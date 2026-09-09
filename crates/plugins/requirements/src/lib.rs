//! pip requirements file file type plugin: core and presentation halves.
//!
//! A requirements file names what to install and, when it is doing its
//! job, exactly which version. This reads the names, the specifiers, the
//! extras, the markers, the includes, the index addresses and the hash
//! pins, and says which lines are not pinned to one version.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &[];

/// The backslash pip lets a requirement continue over.
const CONTINUATION: char = '\\';

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One requirement line, after its continuations are joined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirement {
    /// The distribution's name.
    pub name: String,
    /// Its version specifier, empty when the line names no version.
    pub specifier: String,
    /// The extras asked for in square brackets.
    pub extras: Vec<String>,
    /// The environment marker after the semicolon, when there is one.
    pub marker: Option<String>,
    /// How many `--hash=` pins the line carries.
    pub hashes: usize,
    /// Whether the line is an editable install.
    pub editable: bool,
}

/// View data produced by [`RequirementsCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementsView {
    /// Every requirement, in the order the file lists them.
    pub requirements: Vec<Requirement>,
    /// The files pulled in with `-r`.
    pub includes: Vec<String>,
    /// The files pulled in with `-c`.
    pub constraints: Vec<String>,
    /// The package index addresses the file names.
    pub indexes: Vec<String>,
    /// Requirements with no `==`, so two installs a week apart can differ.
    pub unpinned: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The comparison operators a specifier may open with, longest first so
/// `===` is not read as `==` with a stray `=`.
const OPERATORS: &[&str] = &["===", "==", "!=", "<=", ">=", "~=", "<", ">"];

/// The option prefixes that name a file or an index rather than a package.
const FILE_OPTIONS: &[(&str, &str)] = &[
    ("-r ", "include"),
    ("--requirement ", "include"),
    ("-c ", "constraint"),
    ("--constraint ", "constraint"),
    ("-i ", "index"),
    ("--index-url ", "index"),
    ("--extra-index-url ", "index"),
    ("--find-links ", "index"),
];

/// `text` with comments dropped and backslash continuations joined.
///
/// pip lets a requirement run over several lines, and puts each `--hash`
/// on its own; read line by line those look like requirements of their
/// own, and the pins land on nothing.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut pending = String::new();
    for raw in text.lines() {
        let line = strip_comment(raw);
        let line = line.trim();
        if line.is_empty() && pending.is_empty() {
            continue;
        }
        if let Some(head) = line.strip_suffix(CONTINUATION) {
            pending.push_str(head.trim_end());
            pending.push(' ');
            continue;
        }
        pending.push_str(line);
        if !pending.trim().is_empty() {
            lines.push(pending.trim().to_owned());
        }
        pending.clear();
    }
    if !pending.trim().is_empty() {
        lines.push(pending.trim().to_owned());
    }
    lines
}

/// `line` up to its comment, if it has one.
///
/// A `#` only opens a comment at the start of the line or after a space:
/// inside a URL fragment - `...#egg=name` - it is part of the address.
fn strip_comment(line: &str) -> &str {
    if line.trim_start().starts_with('#') {
        return "";
    }
    match line.find(" #") {
        Some(at) => &line[..at],
        None => line,
    }
}

/// The extras inside `[...]`, and `rest` with the brackets removed.
fn extras_of(rest: &str) -> (Vec<String>, String) {
    let Some(open) = rest.find('[') else {
        return (Vec::new(), rest.to_owned());
    };
    let Some(close) = rest[open..].find(']').map(|at| at + open) else {
        return (Vec::new(), rest.to_owned());
    };
    let extras = rest[open + 1..close]
        .split(',')
        .map(str::trim)
        .filter(|extra| !extra.is_empty())
        .map(str::to_owned)
        .collect();
    (extras, format!("{}{}", &rest[..open], &rest[close + 1..]))
}

/// The requirement `line` states, if it states one.
fn requirement(line: &str) -> Option<Requirement> {
    let editable = line.starts_with("-e ") || line.starts_with("--editable ");
    let body = line
        .strip_prefix("-e ")
        .or_else(|| line.strip_prefix("--editable "))
        .unwrap_or(line);

    // The hash pins can sit anywhere on the logical line.
    let hashes = body
        .split_whitespace()
        .filter(|word| word.starts_with("--hash"))
        .count();
    let body: String = body
        .split_whitespace()
        .filter(|word| !word.starts_with("--hash"))
        .collect::<Vec<_>>()
        .join(" ");

    let (head, marker) = match body.split_once(';') {
        Some((head, marker)) => (head.trim().to_owned(), Some(marker.trim().to_owned())),
        None => (body.trim().to_owned(), None),
    };
    if head.is_empty() {
        return None;
    }
    let (extras, head) = extras_of(&head);

    let at = OPERATORS
        .iter()
        .filter_map(|operator| head.find(operator))
        .min()
        .unwrap_or(head.len());
    let name = head[..at].trim().to_owned();
    let specifier = head[at..].trim().to_owned();
    if name.is_empty() {
        return None;
    }
    Some(Requirement {
        name,
        specifier,
        extras,
        marker,
        hashes,
        editable,
    })
}

/// Which of the three lists `line` belongs on, if it belongs on one.
fn file_option(line: &str) -> Option<(&'static str, String)> {
    FILE_OPTIONS.iter().find_map(|(prefix, kind)| {
        line.strip_prefix(prefix)
            .map(|value| (*kind, value.trim().to_owned()))
    })
}

/// Everything [`RequirementsView`] holds, read from `text`.
fn parse(text: &str) -> RequirementsView {
    let mut view = RequirementsView {
        requirements: Vec::new(),
        includes: Vec::new(),
        constraints: Vec::new(),
        indexes: Vec::new(),
        unpinned: Vec::new(),
        truncated: false,
    };
    for line in logical_lines(text) {
        if let Some((kind, value)) = file_option(&line) {
            match kind {
                "include" => view.includes.push(value),
                "constraint" => view.constraints.push(value),
                _ => view.indexes.push(value),
            }
            continue;
        }
        if line.starts_with('-') && !line.starts_with("-e ") && !line.starts_with("--editable ") {
            // Some other option - `--no-binary`, `--pre` - which names no
            // package and belongs to none.
            continue;
        }
        if let Some(requirement) = requirement(&line) {
            // An editable install is a working copy, so "unpinned" says
            // nothing about it that the reader does not already know.
            if !requirement.editable && !requirement.specifier.contains("==") {
                view.unpinned.push(requirement.name.clone());
            }
            view.requirements.push(requirement);
        }
    }
    view
}

/// Whether `text` is a pip requirements file.
fn looks_like_it(text: &str) -> bool {
    let lines = logical_lines(text);
    if lines.is_empty() {
        return false;
    }
    let mut pinned_or_option = false;
    for line in &lines {
        if file_option(line).is_some()
            || line.starts_with("-e ")
            || line.starts_with("--editable ")
            || line.starts_with("--")
        {
            pinned_or_option = true;
            continue;
        }
        // Anything else has to read as a requirement, or this is prose
        // that happens to mention a version.
        let Some(requirement) = requirement(line) else {
            return false;
        };
        if !requirement
            .name
            .chars()
            .all(|letter| letter.is_ascii_alphanumeric() || "._-".contains(letter))
        {
            return false;
        }
        if !requirement.specifier.is_empty() || requirement.hashes > 0 {
            pinned_or_option = true;
        }
    }
    // A bare list of words is a list of words. Something has to mark it
    // out as a requirements file: a version, a hash, or an option.
    pinned_or_option
}

/// The pip requirements file plugin's core half.
#[derive(Debug, Default)]
pub struct RequirementsCore;

impl PluginCore for RequirementsCore {
    fn name(&self) -> &'static str {
        "requirements"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A requirements file is text, and `text` recognises any valid
        // UTF-8 - including this. Without saying so, the file falls to
        // whichever plugin happens to sit earliest in the list.
        &["text"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The lines are the whole of the file, and every one of them is
        // on the view already, so no `content` here.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The pip requirements file plugin's presentation half.
#[derive(Debug, Default)]
pub struct RequirementsPresentation;

impl PluginPresentation for RequirementsPresentation {
    fn name(&self) -> &'static str {
        "requirements"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PIP",
            tint: 0x0030_6998,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: RequirementsView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        lines.push(format!("{} requirement(s):", view.requirements.len()));
        for requirement in &view.requirements {
            let extras = if requirement.extras.is_empty() {
                String::new()
            } else {
                format!("[{}]", requirement.extras.join(","))
            };
            let editable = if requirement.editable {
                " (editable)"
            } else {
                ""
            };
            let specifier = if requirement.specifier.is_empty() {
                "any version"
            } else {
                &requirement.specifier
            };
            lines.push(format!(
                "  {}{extras} {specifier}{editable}",
                requirement.name
            ));
            if let Some(marker) = &requirement.marker {
                lines.push(format!("      only when {marker}"));
            }
            if requirement.hashes > 0 {
                lines.push(format!("      {} hash pin(s)", requirement.hashes));
            }
        }
        if !view.includes.is_empty() {
            lines.push(format!("Includes: {}", view.includes.join(", ")));
        }
        if !view.constraints.is_empty() {
            lines.push(format!("Constraints: {}", view.constraints.join(", ")));
        }
        if !view.indexes.is_empty() {
            lines.push(format!("Indexes: {}", view.indexes.join(", ")));
        }
        if !view.unpinned.is_empty() {
            lines.push("Not pinned to one version, so two installs a week apart".to_owned());
            lines.push("can bring in different code:".to_owned());
            for name in &view.unpinned {
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
    use super::{RequirementsCore, RequirementsPresentation, RequirementsView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const FILE: &str = concat!(
        "# Production.\n",
        "-r base.txt\n",
        "-c constraints.txt\n",
        "--index-url https://pypi.example.com/simple\n",
        "requests[security,socks]==2.31.0 ; python_version >= \"3.8\"\n",
        "flask>=2.0\n",
        "-e ./local-package\n",
        "urllib3==2.2.1 \\\n",
        "    --hash=sha256:aaaa \\\n",
        "    --hash=sha256:bbbb\n",
    );

    #[test]
    fn sniffs_a_requirements_file() {
        assert!(RequirementsCore.sniff(FILE.as_bytes()));
    }

    #[test]
    fn does_not_claim_a_bare_list_of_words() {
        // Valid as a requirements file, indistinguishable from a list.
        assert!(!RequirementsCore.sniff(b"alpha\nbeta\ngamma\n"));
        assert!(!RequirementsCore.sniff(b""));
    }

    #[test]
    fn does_not_claim_prose_that_mentions_a_version() {
        assert!(!RequirementsCore.sniff(b"We moved to requests==2.31.0 and it broke.\n"));
    }

    #[test]
    fn it_says_it_specialises_text() {
        assert_eq!(RequirementsCore.specialises(), &["text"]);
    }

    #[test]
    fn a_continued_line_keeps_its_hash_pins() {
        let view = parse(FILE);

        let urllib3 = view
            .requirements
            .iter()
            .find(|requirement| requirement.name == "urllib3")
            .expect("urllib3 is one requirement, not a requirement and two stray lines");
        assert_eq!(urllib3.hashes, 2);
        assert_eq!(
            view.requirements.len(),
            4,
            "the hash lines are part of urllib3, not requirements of their own"
        );
    }

    #[test]
    fn reads_extras_and_the_environment_marker() {
        let view = parse(FILE);

        let requests = &view.requirements[0];
        assert_eq!(requests.name, "requests");
        assert_eq!(requests.specifier, "==2.31.0");
        assert_eq!(
            requests.extras,
            vec!["security".to_owned(), "socks".to_owned()]
        );
        assert_eq!(
            requests.marker.as_deref(),
            Some("python_version >= \"3.8\"")
        );
    }

    #[test]
    fn sorts_the_options_by_what_they_name() {
        let view = parse(FILE);

        assert_eq!(view.includes, vec!["base.txt".to_owned()]);
        assert_eq!(view.constraints, vec!["constraints.txt".to_owned()]);
        assert_eq!(
            view.indexes,
            vec!["https://pypi.example.com/simple".to_owned()]
        );
    }

    #[test]
    fn names_what_is_not_pinned_and_spares_the_editable_install() {
        let view = parse(FILE);

        assert_eq!(
            view.unpinned,
            vec!["flask".to_owned()],
            "an editable install is a working copy; pinning says nothing about it"
        );
    }

    #[test]
    fn a_url_fragment_is_not_a_comment() {
        let view = parse("thing @ https://example.com/t.zip#egg=thing\n");

        assert_eq!(view.requirements.len(), 1);
        assert_eq!(
            view.requirements[0].name,
            "thing @ https://example.com/t.zip#egg=thing"
        );
    }

    #[test]
    fn presents_the_unpinned_warning_with_its_reason() {
        let data = serde_json::to_value(parse(FILE)).unwrap();

        let lines = RequirementsPresentation.present(&data);

        assert_eq!(lines[0], "4 requirement(s):");
        assert!(
            lines
                .iter()
                .any(|line| line.contains("two installs a week apart"))
        );
        assert!(lines.iter().any(|line| line.contains("2 hash pin(s)")));
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/requirements/requirements.txt");

        let data = RequirementsCore.view(&path).unwrap();
        let view: RequirementsView = serde_json::from_value(data).unwrap();

        assert!(view.requirements.len() >= 5);
        assert!(!view.includes.is_empty());
        assert!(!view.constraints.is_empty());
        assert!(!view.indexes.is_empty());
        assert!(!view.unpinned.is_empty());
        assert!(view.requirements.iter().any(|r| !r.extras.is_empty()));
        assert!(view.requirements.iter().any(|r| r.marker.is_some()));
        assert!(view.requirements.iter().any(|r| r.hashes > 0));
        assert!(view.requirements.iter().any(|r| r.editable));
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::RequirementsCore),
            plugin_api::PluginPresentation::extensions(&crate::RequirementsPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
