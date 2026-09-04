//! Vim script file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Substrings that mark a file as Vim script — a `vim9script` header, the
/// `function!`/`endfunction` declaration pair, an `autocmd` handler, a
/// `noremap` key mapping (matching all of `nnoremap`/`inoremap`/`vnoremap`/
/// `cnoremap`/`onoremap`/`xnoremap`/bare `noremap`), a `colorscheme`
/// switch, `set nocompatible`, `syntax on`/`syntax enable`,
/// `filetype plugin indent on`, or a `let g:` global-variable assignment —
/// none used by any sibling plugin.
const MARKERS: &[&str] = &[
    "vim9script",
    "function!",
    "endfunction",
    "autocmd",
    "noremap ",
    "colorscheme ",
    "set nocompatible",
    "syntax on",
    "syntax enable",
    "filetype plugin indent on",
    "let g:",
];

/// View data produced by [`VimscriptCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VimscriptView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names from `function!`/`function` declarations found in the content.
    pub functions: Vec<String>,
    /// Names from `let g:name` global-variable assignments found in the
    /// content.
    pub globals: Vec<String>,
}

/// Extracts the function name from a `function! Name(...)` or
/// `function Name(...)` declaration line, or `None` if `trimmed` is not
/// such a declaration.
fn parse_function_name(trimmed: &str) -> Option<&str> {
    let rest = if let Some(rest) = trimmed.strip_prefix("function!") {
        rest
    } else {
        let rest = trimmed.strip_prefix("function")?;
        rest.starts_with(char::is_whitespace).then_some(rest)?
    };
    let rest = rest.trim_start();
    let end = rest.find('(')?;
    let name = rest[..end].trim();
    (!name.is_empty()).then_some(name)
}

/// Extracts the variable name from a `let g:name = ...` global-variable
/// assignment line, or `None` if `trimmed` is not such an assignment.
fn parse_global_name(trimmed: &str) -> Option<&str> {
    let rest = trimmed.strip_prefix("let ")?.trim_start();
    let rest = rest.strip_prefix("g:")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses `function!`/`function` and `let g:` declarations out of
/// `content`, in source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut globals = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(name) = parse_function_name(trimmed) {
            functions.push(name.to_owned());
        } else if let Some(name) = parse_global_name(trimmed) {
            globals.push(name.to_owned());
        }
    }
    (functions, globals)
}

/// Whether `text` looks like Vim script source, per [`MARKERS`].
fn has_vimscript_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// The Vim script plugin's core half.
#[derive(Debug, Default)]
pub struct VimscriptCore;

impl PluginCore for VimscriptCore {
    fn name(&self) -> &'static str {
        "vimscript"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_vimscript_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, globals) = parse_definitions(&content);
        let view = VimscriptView {
            content,
            truncated,
            functions,
            globals,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Vim script plugin's presentation half.
#[derive(Debug, Default)]
pub struct VimscriptPresentation;

impl PluginPresentation for VimscriptPresentation {
    fn name(&self) -> &'static str {
        "vimscript"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: VimscriptView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.globals.is_empty() {
            lines.push(format!("globals: {}", view.globals.join(", ")));
        }
        if !view.functions.is_empty() {
            lines.push(format!("functions: {}", view.functions.join(", ")));
        }
        lines.extend(view.content.lines().map(str::to_owned));
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, VimscriptCore, VimscriptPresentation, VimscriptView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-vimscript-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_vimscript_markers_as_vimscript() {
        assert!(VimscriptCore.sniff(b"vim9script\n\nvar x = 1\n"));
        assert!(VimscriptCore.sniff(b"function! Greet()\n  echo \"hi\"\nendfunction\n"));
        assert!(VimscriptCore.sniff(b"autocmd BufWritePre * :call TrimWhitespace()\n"));
        assert!(VimscriptCore.sniff(b"nnoremap <leader>f :Files<CR>\n"));
        assert!(VimscriptCore.sniff(b"colorscheme desert\n"));
        assert!(VimscriptCore.sniff(b"set nocompatible\nfiletype plugin on\n"));
        assert!(VimscriptCore.sniff(b"syntax on\n"));
        assert!(VimscriptCore.sniff(b"filetype plugin indent on\n"));
        assert!(VimscriptCore.sniff(b"let g:mapleader = \"\\<Space>\"\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_vimscript() {
        assert!(!VimscriptCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!VimscriptCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!VimscriptCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!VimscriptCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!VimscriptCore.sniff(b"program greeter\n    implicit none\nend program greeter\n"));
        assert!(!VimscriptCore.sniff(b".global _start\n_start:\n    movq $1, %rax\n"));
        assert!(!VimscriptCore.sniff(b"if [ -f foo ]; then\n  echo hi\nfi\n"));
        assert!(!VimscriptCore.sniff(b"just a regular line of text\n"));
        assert!(!VimscriptCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_vimscript_file_and_extracts_definitions() {
        let path = unique_temp_file("vimrc.vim");
        std::fs::write(
            &path,
            "set nocompatible\nlet g:mapleader = \",\"\nlet g:loaded_plugin = 1\n\nfunction! TrimWhitespace()\n  let l:save = winsaveview()\n  %s/\\s\\+$//e\n  call winrestview(l:save)\nendfunction\n\nautocmd BufWritePre * call TrimWhitespace()\n",
        )
        .unwrap();

        let data = VimscriptCore.view(&path).unwrap();
        let view: VimscriptView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.globals, vec!["mapleader", "loaded_plugin"]);
        assert_eq!(view.functions, vec!["TrimWhitespace"]);
        assert!(view.content.contains("BufWritePre"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.vim");
        let mut content = "let g:mapleader = \",\"\n".to_owned();
        content.push_str(&"    \" a comment line\n".repeat(MAX_VIEW_BYTES));
        std::fs::write(&path, content).unwrap();

        let data = VimscriptCore.view(&path).unwrap();
        let view: VimscriptView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_globals_functions_and_content() {
        let data = serde_json::to_value(VimscriptView {
            content: "function! Greet()\n  echo \"hi\"\nendfunction".to_owned(),
            truncated: false,
            functions: vec!["Greet".to_owned()],
            globals: vec!["mapleader".to_owned()],
        })
        .unwrap();

        let lines = VimscriptPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "globals: mapleader",
                "functions: Greet",
                "function! Greet()",
                "  echo \"hi\"",
                "endfunction"
            ]
        );
    }
}
