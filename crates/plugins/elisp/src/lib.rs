//! Emacs Lisp file type plugin: core and presentation halves.
//!
//! An Emacs Lisp file is a package: a header, some definitions, and a
//! `provide` at the end. This reads the header, whether it asks for
//! lexical binding, every definition with whether it is a command and
//! whether it is autoloaded, the features required and provided, the key
//! bindings - and the commands nobody can reach until something else has
//! loaded the file.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["el"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One definition the file makes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    /// Its name.
    pub name: String,
    /// The form that defined it: `defun`, `defvar`, `defcustom` and so on.
    pub form: String,
    /// Whether a function is a command a person can run by name.
    pub interactive: bool,
    /// Whether an `;;;###autoload` cookie sits above it, which is what
    /// makes it reachable before the package has been loaded.
    pub autoload: bool,
}

/// View data produced by [`ElispCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElispView {
    /// The package's name and one-line summary from the first line.
    pub package: Option<String>,
    /// Whether the file asks for lexical binding.
    pub lexical_binding: bool,
    /// Every definition, in the order they are written.
    pub definitions: Vec<Definition>,
    /// The features pulled in with `require`.
    pub requires: Vec<String>,
    /// The feature this file provides.
    pub provides: Option<String>,
    /// The keys bound, as `key -> command`.
    pub key_bindings: Vec<String>,
    /// Commands with no `;;;###autoload` cookie, which a person cannot
    /// run until something else has loaded this file first.
    pub commands_without_autoload: Vec<String>,
    /// Whether the file was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The defining forms worth recording, and whether each makes a function.
const FORMS: &[(&str, bool)] = &[
    ("defun", true),
    ("defmacro", true),
    ("defsubst", true),
    ("cl-defun", true),
    ("define-minor-mode", true),
    ("define-derived-mode", true),
    ("defvar", false),
    ("defvar-local", false),
    ("defconst", false),
    ("defcustom", false),
    ("defface", false),
    ("defgroup", false),
];

/// The forms that bind a key, each taking the key as its second argument.
const BINDERS: &[&str] = &["define-key", "keymap-set", "global-set-key", "bind-key"];

/// The first single- or double-quoted or bare word after `at` in `line`.
fn word_after(line: &str, at: usize) -> Option<String> {
    let rest = line[at..].trim_start();
    let rest = rest.strip_prefix('\'').unwrap_or(rest);
    let word: String = rest
        .chars()
        .take_while(|letter| !letter.is_whitespace() && *letter != ')' && *letter != '(')
        .collect();
    (!word.is_empty()).then_some(word)
}

/// The form and name a definition line opens with.
fn definition_of(line: &str) -> Option<(String, String, bool)> {
    let rest = line.trim().strip_prefix('(')?;
    let form = rest
        .split([' ', '\t', '\n', ')'])
        .next()
        .filter(|word| !word.is_empty())?;
    let (_, makes_function) = FORMS.iter().find(|(name, _)| *name == form)?;
    let name = word_after(rest, form.len())?;
    Some((form.to_owned(), name, *makes_function))
}

/// Everything [`ElispView`] holds, read from `text`.
fn parse(text: &str) -> ElispView {
    let mut view = ElispView {
        package: None,
        lexical_binding: text.contains("lexical-binding: t"),
        definitions: Vec::new(),
        requires: Vec::new(),
        provides: None,
        key_bindings: Vec::new(),
        commands_without_autoload: Vec::new(),
        truncated: false,
    };
    // An autoload cookie applies to the next definition, and the body of
    // a definition is read until the next one starts.
    let mut cookie = false;
    let mut current: Option<usize> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if view.package.is_none()
            && let Some(rest) = line.strip_prefix(";;; ")
            && rest.contains("---")
        {
            view.package = Some(rest.trim().to_owned());
            continue;
        }
        if line.starts_with(";;;###autoload") {
            cookie = true;
            continue;
        }
        if line.starts_with(';') {
            continue;
        }
        if let Some(at) = line.find("(require ") {
            if let Some(feature) = word_after(line, at + "(require ".len()) {
                view.requires.push(feature);
            }
            continue;
        }
        if let Some(at) = line.find("(provide ") {
            view.provides = word_after(line, at + "(provide ".len());
            continue;
        }
        for binder in BINDERS {
            let opener = format!("({binder} ");
            let Some(at) = line.find(&opener) else {
                continue;
            };
            let arguments = line[at + opener.len()..].trim();
            // `(define-key map (kbd "C-c s") #'command)` - the map comes
            // first for some binders and not for others, so take the
            // quoted key and the last symbol on the line.
            let key = arguments
                .find('"')
                .and_then(|open| {
                    arguments[open + 1..]
                        .find('"')
                        .map(|end| arguments[open + 1..open + 1 + end].to_owned())
                })
                .unwrap_or_else(|| "?".to_owned());
            let command = line
                .rsplit(['#', '\''])
                .next()
                .unwrap_or("")
                .trim_end_matches(')')
                .trim()
                .to_owned();
            view.key_bindings.push(format!("{key} -> {command}"));
        }

        if let Some((form, name, makes_function)) = definition_of(line) {
            view.definitions.push(Definition {
                name,
                form,
                interactive: false,
                autoload: cookie,
            });
            cookie = false;
            current = makes_function.then(|| view.definitions.len() - 1);
            continue;
        }
        if line.starts_with("(interactive")
            && let Some(at) = current
        {
            view.definitions[at].interactive = true;
        }
    }

    view.commands_without_autoload = view
        .definitions
        .iter()
        .filter(|definition| definition.interactive && !definition.autoload)
        .map(|definition| definition.name.clone())
        .collect();
    view
}

/// Whether `text` is Emacs Lisp.
fn looks_like_it(text: &str) -> bool {
    let view = parse(text);
    // Brackets and `defun` are Lisp generally. The Emacs dialect announces
    // itself with a lexical-binding header, an autoload cookie, a
    // `provide`, or one of the forms only Emacs has.
    let emacs_shaped = text.contains("lexical-binding")
        || text.contains(";;;###autoload")
        || view.provides.is_some()
        || view.definitions.iter().any(|definition| {
            definition.form.starts_with("defcustom") || definition.form.starts_with("define-")
        });
    emacs_shaped && !view.definitions.is_empty()
}

/// The Emacs Lisp plugin's core half.
#[derive(Debug, Default)]
pub struct ElispCore;

impl PluginCore for ElispCore {
    fn name(&self) -> &'static str {
        "elisp"
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
        // The definitions are what a reader came for; the bodies read
        // better in the file itself.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Emacs Lisp plugin's presentation half.
#[derive(Debug, Default)]
pub struct ElispPresentation;

impl PluginPresentation for ElispPresentation {
    fn name(&self) -> &'static str {
        "elisp"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "EL",
            tint: 0x007f_5ab6,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: ElispView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(package) = &view.package {
            lines.push(package.clone());
        }
        if !view.lexical_binding {
            lines.push("No lexical-binding header, so every `let` is dynamic and".to_owned());
            lines.push("a closure does not close over anything:".to_owned());
            lines.push("  add `;;; -*- lexical-binding: t -*-` to the first line".to_owned());
        }
        if !view.requires.is_empty() {
            lines.push(format!("Requires: {}", view.requires.join(", ")));
        }
        if let Some(provides) = &view.provides {
            lines.push(format!("Provides: {provides}"));
        }
        lines.push(format!("{} definition(s):", view.definitions.len()));
        for definition in &view.definitions {
            let mut notes = Vec::new();
            if definition.interactive {
                notes.push("command".to_owned());
            }
            if definition.autoload {
                notes.push("autoloaded".to_owned());
            }
            let notes = if notes.is_empty() {
                String::new()
            } else {
                format!("  [{}]", notes.join(", "))
            };
            lines.push(format!("  {} {}{notes}", definition.form, definition.name));
        }
        if !view.key_bindings.is_empty() {
            lines.push(format!("{} key binding(s):", view.key_bindings.len()));
            for binding in &view.key_bindings {
                lines.push(format!("  {binding}"));
            }
        }
        if !view.commands_without_autoload.is_empty() {
            lines.push("No autoload cookie, so these cannot be run by name until".to_owned());
            lines.push("something else has already loaded this file:".to_owned());
            for name in &view.commands_without_autoload {
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
    use super::{ElispCore, ElispPresentation, ElispView, definition_of, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const PACKAGE: &str = concat!(
        ";;; csvstats.el --- Summary statistics for a region -*- lexical-binding: t -*-\n",
        "\n",
        ";; Author: The floor\n",
        "\n",
        ";;; Code:\n",
        "\n",
        "(require 'cl-lib)\n",
        "(require 'subr-x)\n",
        "\n",
        "(defgroup csvstats nil\n",
        "  \"Summary statistics.\"\n",
        "  :group 'tools)\n",
        "\n",
        "(defcustom csvstats-separator \",\"\n",
        "  \"What separates one field from the next.\"\n",
        "  :type 'string)\n",
        "\n",
        "(defvar csvstats--cache nil\n",
        "  \"Nothing here yet.\")\n",
        "\n",
        ";;;###autoload\n",
        "(defun csvstats-mean (beginning end)\n",
        "  \"Average the numbers between BEGINNING and END.\"\n",
        "  (interactive \"r\")\n",
        "  (message \"%s\" (csvstats--average beginning end)))\n",
        "\n",
        "(defun csvstats-deviation (beginning end)\n",
        "  \"Spread of the numbers between BEGINNING and END.\"\n",
        "  (interactive \"r\")\n",
        "  (message \"%s\" (csvstats--spread beginning end)))\n",
        "\n",
        "(defun csvstats--average (beginning end)\n",
        "  \"Not a command: no interactive form.\"\n",
        "  (/ (apply #'+ (csvstats--numbers beginning end)) 1.0))\n",
        "\n",
        "(defvar csvstats-mode-map\n",
        "  (let ((map (make-sparse-keymap)))\n",
        "    (define-key map (kbd \"C-c s m\") #'csvstats-mean)\n",
        "    (define-key map (kbd \"C-c s d\") #'csvstats-deviation)\n",
        "    map))\n",
        "\n",
        ";;;###autoload\n",
        "(define-minor-mode csvstats-mode\n",
        "  \"Summary statistics in this buffer.\"\n",
        "  :lighter \" CSV\")\n",
        "\n",
        "(provide 'csvstats)\n",
        ";;; csvstats.el ends here\n",
    );

    #[test]
    fn sniffs_a_package() {
        assert!(ElispCore.sniff(PACKAGE.as_bytes()));
    }

    #[test]
    fn does_not_claim_lisp_that_is_not_the_emacs_dialect() {
        assert!(!ElispCore.sniff(b"(defun add (a b)\n  (+ a b))\n"));
        assert!(!ElispCore.sniff(b""));
    }

    #[test]
    fn reads_the_header_and_the_lexical_binding_promise() {
        let view = parse(PACKAGE);

        assert!(view.lexical_binding);
        assert!(
            view.package
                .as_deref()
                .is_some_and(|said| said.contains("csvstats.el ---")),
            "the first line is the package's name and summary"
        );
        assert_eq!(view.provides.as_deref(), Some("csvstats"));
        assert_eq!(
            view.requires,
            vec!["cl-lib".to_owned(), "subr-x".to_owned()]
        );
    }

    #[test]
    fn a_defining_form_is_told_from_a_call() {
        assert!(definition_of("(defun csvstats-mean (beginning end)").is_some());
        assert!(definition_of("(defcustom csvstats-separator \",\"").is_some());
        assert!(
            definition_of("(message \"%s\" x)").is_none(),
            "an ordinary call is not a definition"
        );
    }

    #[test]
    fn an_interactive_form_makes_a_command() {
        let view = parse(PACKAGE);

        let mean = view
            .definitions
            .iter()
            .find(|d| d.name == "csvstats-mean")
            .unwrap();
        assert!(mean.interactive);
        assert!(mean.autoload);
        let average = view
            .definitions
            .iter()
            .find(|d| d.name == "csvstats--average")
            .unwrap();
        assert!(
            !average.interactive,
            "a docstring mentioning commands does not make one"
        );
    }

    #[test]
    fn a_cookie_applies_to_the_definition_below_it() {
        let view = parse(PACKAGE);

        let mode = view
            .definitions
            .iter()
            .find(|d| d.name == "csvstats-mode")
            .unwrap();
        assert!(mode.autoload);
        let deviation = view
            .definitions
            .iter()
            .find(|d| d.name == "csvstats-deviation")
            .unwrap();
        assert!(
            !deviation.autoload,
            "the cookie above `mean` is spent on `mean`"
        );
    }

    #[test]
    fn reads_the_key_bindings() {
        let view = parse(PACKAGE);

        assert_eq!(
            view.key_bindings,
            vec![
                "C-c s m -> csvstats-mean".to_owned(),
                "C-c s d -> csvstats-deviation".to_owned()
            ]
        );
    }

    #[test]
    fn names_the_command_nobody_can_reach_yet() {
        let view = parse(PACKAGE);

        assert_eq!(
            view.commands_without_autoload,
            vec!["csvstats-deviation".to_owned()],
            "`csvstats-mean` has a cookie; this one has not"
        );
    }

    #[test]
    fn presents_the_missing_cookie_with_its_reason() {
        let data = serde_json::to_value(parse(PACKAGE)).unwrap();

        let lines = ElispPresentation.present(&data);

        assert!(
            lines
                .iter()
                .any(|line| line.contains("already loaded this file"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("[command, autoloaded]"))
        );
    }

    #[test]
    fn warns_when_the_file_is_dynamically_bound() {
        let view = parse(concat!(
            ";;; old.el --- An older package\n",
            ";;;###autoload\n",
            "(defun old-thing () (interactive) nil)\n",
            "(provide 'old)\n",
        ));

        assert!(!view.lexical_binding);
        let data = serde_json::to_value(&view).unwrap();
        let lines = ElispPresentation.present(&data);
        assert!(
            lines
                .iter()
                .any(|line| line.contains("every `let` is dynamic"))
        );
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/elisp/csvstats.el");

        let data = ElispCore.view(&path).unwrap();
        let view: ElispView = serde_json::from_value(data).unwrap();

        assert!(view.package.is_some());
        assert!(view.lexical_binding);
        assert!(view.definitions.len() >= 8);
        assert!(view.definitions.iter().any(|d| d.interactive));
        assert!(view.definitions.iter().any(|d| d.autoload));
        assert!(view.definitions.iter().any(|d| d.form == "defcustom"));
        assert!(view.requires.len() >= 2);
        assert!(view.provides.is_some());
        assert!(view.key_bindings.len() >= 2);
        assert!(!view.commands_without_autoload.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::ElispCore),
            plugin_api::PluginPresentation::extensions(&crate::ElispPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
