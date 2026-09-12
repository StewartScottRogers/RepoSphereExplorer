//! MATLAB file type plugin: core and presentation halves.
//!
//! A MATLAB file is a script, a function file or a class, and the `.m`
//! extension it wears belongs to Objective-C as well - so this is
//! settled on content. It reads the functions with their inputs and
//! several outputs, which of them are nested, the cell sections, the
//! class, the calls that need a toolbox installed, and the functions
//! that take inputs and check none of them.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One function the file declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatlabFunction {
    /// Its name.
    pub name: String,
    /// What it takes.
    pub inputs: Vec<String>,
    /// What it gives back. MATLAB allows several.
    pub outputs: Vec<String>,
    /// Whether it is nested inside another function rather than being a
    /// local one at the end of the file.
    pub nested: bool,
    /// Whether it opens with an `arguments` block, which is how MATLAB
    /// checks what it was handed.
    pub validates: bool,
}

/// View data produced by [`MatlabCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatlabView {
    /// `script`, `function file` or `class`.
    pub kind: String,
    /// The class the file defines, when it defines one.
    pub class: Option<String>,
    /// Every function, in the order they are written.
    pub functions: Vec<MatlabFunction>,
    /// The `%%` cell sections, by their titles.
    pub sections: Vec<String>,
    /// Calls to functions that live in a toolbox rather than in MATLAB
    /// itself, so the file will not run without that toolbox installed.
    pub toolbox_calls: Vec<String>,
    /// Functions that take inputs and check none of them.
    pub unvalidated: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// Functions that come from a toolbox, with the toolbox each needs.
///
/// A small list on purpose: it names the ones a reader is most likely to
/// meet, rather than pretending to know every toolbox there is.
const TOOLBOX_FUNCTIONS: &[(&str, &str)] = &[
    ("fitlm", "Statistics and Machine Learning"),
    ("fitglm", "Statistics and Machine Learning"),
    ("ranova", "Statistics and Machine Learning"),
    ("kmeans", "Statistics and Machine Learning"),
    ("trainNetwork", "Deep Learning"),
    ("imread", "Image Processing"),
    ("imfilter", "Image Processing"),
    ("imresize", "Image Processing"),
    ("filtfilt", "Signal Processing"),
    ("butter", "Signal Processing"),
    ("periodogram", "Signal Processing"),
    ("tf", "Control System"),
    ("bode", "Control System"),
    ("fmincon", "Optimization"),
    ("linprog", "Optimization"),
    ("sym", "Symbolic Math"),
    ("solve", "Symbolic Math"),
    ("parfor", "Parallel Computing"),
];

/// `line` with its comment stripped.
///
/// A `%` opens a comment unless it is inside a string, and `%%` at the
/// start of a line opens a section rather than an ordinary comment.
fn cleaned(line: &str) -> &str {
    let mut in_string = false;
    for (at, letter) in line.char_indices() {
        match letter {
            '\'' | '"' => in_string = !in_string,
            '%' if !in_string => return &line[..at],
            _ => {}
        }
    }
    line
}

/// The names between brackets or square brackets, split on commas.
fn names_in(text: &str) -> Vec<String> {
    text.trim()
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')'])
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The function `line` declares, if it declares one.
fn function_of(line: &str, nested: bool) -> Option<MatlabFunction> {
    let rest = line.trim().strip_prefix("function")?;
    if !rest.starts_with([' ', '\t', '[']) {
        return None;
    }
    let rest = rest.trim();
    // `function [a, b] = name(x)`, `function a = name(x)`, `function name(x)`.
    let (outputs, tail) = match rest.split_once('=') {
        Some((left, right)) if !left.contains('(') => (names_in(left), right.trim()),
        _ => (Vec::new(), rest),
    };
    let name = tail.split('(').next()?.trim().to_owned();
    if name.is_empty() {
        return None;
    }
    let inputs = tail
        .split_once('(')
        .map(|(_, arguments)| names_in(arguments))
        .unwrap_or_default();
    Some(MatlabFunction {
        name,
        inputs,
        outputs,
        nested,
        validates: false,
    })
}

/// Whether `line` calls the function `name`.
///
/// A bare `contains` reads `fprintf(` as a call to `tf`, and `assume(` as
/// a call to `sum`: the name has to start where a name can start.
fn calls(line: &str, name: &str) -> bool {
    let mut from = 0usize;
    while let Some(offset) = line[from..].find(name) {
        let at = from + offset;
        let after = &line[at + name.len()..];
        let starts_a_word = at == 0
            || !line[..at]
                .chars()
                .next_back()
                .is_some_and(|letter| letter.is_alphanumeric() || letter == '_');
        if starts_a_word && (after.starts_with('(') || after.starts_with(' ')) {
            return true;
        }
        from = at + name.len();
    }
    false
}

/// Everything [`MatlabView`] holds, read from `text`.
fn parse(text: &str) -> MatlabView {
    let mut view = MatlabView {
        kind: "script".to_owned(),
        class: None,
        functions: Vec::new(),
        sections: Vec::new(),
        toolbox_calls: Vec::new(),
        unvalidated: Vec::new(),
        truncated: false,
    };
    // How deep in `function`/`if`/`for` blocks the walk is, and the depth
    // each function was opened at, so a nested one can be told from a
    // local one.
    let mut depth = 0usize;
    let mut open_functions: Vec<usize> = Vec::new();

    for raw in text.lines() {
        let trimmed = raw.trim();
        if let Some(title) = trimmed.strip_prefix("%%") {
            view.sections.push(title.trim().to_owned());
            continue;
        }
        let line = cleaned(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("classdef ") {
            "class".clone_into(&mut view.kind);
            view.class = Some(
                rest.split([' ', '<', '('])
                    .next()
                    .unwrap_or(rest)
                    .trim()
                    .to_owned(),
            );
            depth += 1;
            continue;
        }
        for (name, toolbox) in TOOLBOX_FUNCTIONS {
            if !calls(line, name) {
                continue;
            }
            let entry = format!("{name} ({toolbox} Toolbox)");
            if !view.toolbox_calls.contains(&entry) {
                view.toolbox_calls.push(entry);
            }
        }
        if line == "arguments" || line.starts_with("arguments ") {
            if let Some(at) = open_functions.last() {
                view.functions[*at].validates = true;
            }
            depth += 1;
            continue;
        }
        if let Some(function) = function_of(line, !open_functions.is_empty()) {
            if view.functions.is_empty() && view.kind == "script" {
                // A file whose first statement is a function is a function
                // file, whatever else it holds.
                "function file".clone_into(&mut view.kind);
            }
            view.functions.push(function);
            open_functions.push(view.functions.len() - 1);
            depth += 1;
            continue;
        }
        if line == "end" || line.starts_with("end ") || line.starts_with("end;") {
            depth = depth.saturating_sub(1);
            if open_functions.len() > depth {
                open_functions.pop();
            }
            continue;
        }
        if OPENERS.iter().any(|keyword| {
            line == *keyword
                || line.starts_with(&format!("{keyword} "))
                || line.starts_with(&format!("{keyword}("))
        }) {
            depth += 1;
        }
    }

    view.unvalidated = view
        .functions
        .iter()
        .filter(|function| !function.inputs.is_empty() && !function.validates)
        .map(|function| function.name.clone())
        .collect();
    view
}

/// The keywords that open a block MATLAB closes with `end`.
const OPENERS: &[&str] = &[
    "if",
    "for",
    "while",
    "switch",
    "try",
    "parfor",
    "methods",
    "properties",
    "events",
    "enumeration",
    "spmd",
];

/// Whether `text` is MATLAB.
fn looks_like_it(text: &str) -> bool {
    // Objective-C claims the `.m` extension too, so this has to be settled
    // on content alone. Objective-C has `#import`, `@interface` and `@end`;
    // MATLAB has none of them, and has these instead.
    if text.contains("#import") || text.contains("@interface") || text.contains("@end") {
        return false;
    }
    let view = parse(text);
    view.class.is_some()
        || !view.sections.is_empty()
        || view
            .functions
            .iter()
            .any(|function| !function.outputs.is_empty() || !function.inputs.is_empty())
}

/// The MATLAB plugin's core half.
#[derive(Debug, Default)]
pub struct MatlabCore;

impl PluginCore for MatlabCore {
    fn name(&self) -> &'static str {
        "matlab"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The declarations are what a reader came for; the arithmetic
        // reads better in the file itself.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The MATLAB plugin's presentation half.
#[derive(Debug, Default)]
pub struct MatlabPresentation;

impl PluginPresentation for MatlabPresentation {
    fn name(&self) -> &'static str {
        "matlab"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "MAT",
            tint: 0x00d9_5319,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MatlabView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!(
            "MATLAB {}{}",
            view.kind,
            view.class
                .as_ref()
                .map_or_else(String::new, |name| format!(": {name}"))
        ));
        if !view.sections.is_empty() {
            lines.push(format!("{} section(s):", view.sections.len()));
            for title in &view.sections {
                lines.push(format!("  {title}"));
            }
        }
        lines.push(format!("{} function(s):", view.functions.len()));
        for function in &view.functions {
            let outputs = if function.outputs.is_empty() {
                String::new()
            } else {
                format!("[{}] = ", function.outputs.join(", "))
            };
            let nested = if function.nested { " (nested)" } else { "" };
            let checked = if function.validates {
                " (checks its arguments)"
            } else {
                ""
            };
            lines.push(format!(
                "  {outputs}{}({}){nested}{checked}",
                function.name,
                function.inputs.join(", ")
            ));
        }
        if !view.toolbox_calls.is_empty() {
            lines.push("Needs a toolbox installed, or these calls fail at run".to_owned());
            lines.push("time rather than when the file is opened:".to_owned());
            for call in &view.toolbox_calls {
                lines.push(format!("  {call}"));
            }
        }
        if !view.unvalidated.is_empty() {
            lines.push("Take inputs and check none of them, so a wrong type".to_owned());
            lines.push("reaches the arithmetic before anything notices:".to_owned());
            for name in &view.unvalidated {
                lines.push(format!("  {name}"));
            }
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MatlabCore, MatlabPresentation, MatlabView, cleaned, function_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const FUNCTION_FILE: &str = concat!(
        "function [mean_value, deviation] = csvstats(values, options)\n",
        "%CSVSTATS Summary statistics for a column.\n",
        "    arguments\n",
        "        values (1,:) double\n",
        "        options.Trim logical = false\n",
        "    end\n",
        "\n",
        "    mean_value = mean(values);\n",
        "    deviation = spread(values, mean_value);\n",
        "\n",
        "    function s = spread(v, m)\n",
        "        s = sqrt(sum((v - m) .^ 2) / numel(v));\n",
        "    end\n",
        "end\n",
        "\n",
        "function out = tidy(raw)\n",
        "    out = raw(~isnan(raw));\n",
        "end\n",
    );

    const SCRIPT: &str = concat!(
        "%% Load the data\n",
        "raw = readmatrix('samples.csv');\n",
        "\n",
        "%% Fit a model\n",
        "model = fitlm(raw(:, 1), raw(:, 2));\n",
        "percent = 100;  % not a comment marker inside 'a % string'\n",
        "\n",
        "%% Report\n",
        "disp(model);\n",
    );

    #[test]
    fn sniffs_a_function_file_and_a_script() {
        assert!(MatlabCore.sniff(FUNCTION_FILE.as_bytes()));
        assert!(MatlabCore.sniff(SCRIPT.as_bytes()));
    }

    #[test]
    fn does_not_claim_objective_c_which_owns_the_same_extension() {
        assert!(
            !MatlabCore
                .sniff(b"#import <Foundation/Foundation.h>\n@interface Greeter : NSObject\n@end\n")
        );
        assert!(!MatlabCore.sniff(b""));
    }

    #[test]
    fn it_claims_no_extension_because_objective_c_has_it() {
        assert!(
            MatlabCore.extensions().is_empty(),
            "`.m` belongs to Objective-C; this is settled on content"
        );
    }

    #[test]
    fn a_percent_inside_a_string_is_not_a_comment() {
        assert_eq!(
            cleaned("percent = 100;  % not a comment marker inside 'a % string'").trim(),
            "percent = 100;"
        );
        assert_eq!(cleaned("label = 'a % sign';").trim(), "label = 'a % sign';");
    }

    #[test]
    fn reads_several_outputs_and_the_inputs() {
        let declared = function_of(
            "function [mean_value, deviation] = csvstats(values, options)",
            false,
        )
        .unwrap();

        assert_eq!(declared.name, "csvstats");
        assert_eq!(
            declared.outputs,
            vec!["mean_value".to_owned(), "deviation".to_owned()]
        );
        assert_eq!(
            declared.inputs,
            vec!["values".to_owned(), "options".to_owned()]
        );
    }

    #[test]
    fn a_nested_function_is_not_a_local_one() {
        let view = parse(FUNCTION_FILE);

        assert_eq!(view.kind, "function file");
        assert_eq!(view.functions.len(), 3);
        let spread = view.functions.iter().find(|f| f.name == "spread").unwrap();
        assert!(spread.nested, "`spread` is inside `csvstats`");
        let tidy = view.functions.iter().find(|f| f.name == "tidy").unwrap();
        assert!(
            !tidy.nested,
            "`tidy` comes after the first function has ended"
        );
    }

    #[test]
    fn an_arguments_block_counts_as_checking() {
        let view = parse(FUNCTION_FILE);

        let csvstats = &view.functions[0];
        assert!(csvstats.validates);
        assert_eq!(
            view.unvalidated,
            vec!["spread".to_owned(), "tidy".to_owned()],
            "only the first function has an arguments block"
        );
    }

    #[test]
    fn a_toolbox_name_inside_another_word_is_not_a_call() {
        use super::calls;

        assert!(calls("model = fitlm(x, y);", "fitlm"));
        assert!(
            !calls("fprintf('%d rows\n', n);", "tf"),
            "`fprintf(` holds the letters of `tf(` and calls no toolbox"
        );
        assert!(!calls("total = subtotal(x);", "tf"));
        assert!(calls("system = tf(num, den);", "tf"));
    }

    #[test]
    fn reads_the_cell_sections_and_the_toolbox_call() {
        let view = parse(SCRIPT);

        assert_eq!(view.kind, "script");
        assert_eq!(
            view.sections,
            vec![
                "Load the data".to_owned(),
                "Fit a model".to_owned(),
                "Report".to_owned()
            ]
        );
        assert!(
            view.toolbox_calls
                .iter()
                .any(|call| call.starts_with("fitlm")),
            "fitlm needs the Statistics and Machine Learning Toolbox"
        );
    }

    #[test]
    fn presents_both_warnings_with_their_reasons() {
        let data = serde_json::to_value(parse(FUNCTION_FILE)).unwrap();

        let lines = MatlabPresentation.present(&data);

        assert_eq!(lines[0], "MATLAB function file");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("before anything notices"))
        );
        assert!(lines.iter().any(|line| line.contains("(nested)")));
    }

    #[test]
    fn the_repository_fixtures_fill_every_field() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

        let data = MatlabCore
            .view(&here.join("../../../samples/matlab/csvstats.m"))
            .unwrap();
        let view: MatlabView = serde_json::from_value(data).unwrap();
        assert_eq!(view.kind, "function file");
        assert!(view.functions.len() >= 3);
        assert!(view.functions.iter().any(|f| f.nested));
        assert!(view.functions.iter().any(|f| f.validates));
        assert!(view.functions.iter().any(|f| f.outputs.len() >= 2));
        assert!(!view.unvalidated.is_empty());

        let data = MatlabCore
            .view(&here.join("../../../samples/matlab/Column.m"))
            .unwrap();
        let class: MatlabView = serde_json::from_value(data).unwrap();
        assert_eq!(class.kind, "class");
        assert_eq!(class.class.as_deref(), Some("Column"));
        assert!(class.functions.len() >= 4);
        assert!(class.functions.iter().any(|f| f.validates));

        let data = MatlabCore
            .view(&here.join("../../../samples/matlab/analyse.m"))
            .unwrap();
        let script: MatlabView = serde_json::from_value(data).unwrap();
        assert_eq!(script.kind, "script");
        assert!(script.sections.len() >= 3);
        assert!(script.toolbox_calls.len() >= 2);
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::MatlabCore),
            plugin_api::PluginPresentation::extensions(&crate::MatlabPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
