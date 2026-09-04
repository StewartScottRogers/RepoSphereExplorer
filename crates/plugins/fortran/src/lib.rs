//! Fortran file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`FortranCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FortranView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names from top-level `program name` declarations found in the
    /// content.
    pub programs: Vec<String>,
    /// Names from top-level `subroutine name(...)` declarations found in
    /// the content.
    pub subroutines: Vec<String>,
}

/// Whether `line`, once trimmed, starts with `keyword` case-insensitively.
fn starts_with_ci(line: &str, keyword: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.len() >= keyword.len() && trimmed[..keyword.len()].eq_ignore_ascii_case(keyword)
}

/// Extracts the identifier that follows a case-insensitive `keyword` prefix
/// on `line`, e.g. `program hello` with keyword `"program "` yields `hello`.
fn parse_name_after<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    if !starts_with_ci(line, keyword) {
        return None;
    }
    let rest = &line.trim_start()[keyword.len()..];
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level program and subroutine names out of `content`, in
/// source order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut programs = Vec::new();
    let mut subroutines = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_name_after(line, "program ") {
            programs.push(name.to_owned());
        } else if let Some(name) = parse_name_after(line, "subroutine ") {
            subroutines.push(name.to_owned());
        }
    }
    (programs, subroutines)
}

/// Whether `text` looks like Fortran source: the `implicit none` statement,
/// a top-level `program `/`subroutine ` declaration, a top-level `end
/// program`/`end subroutine`/`end function`/`end module` closer, or a
/// `write(*,*)` formatted write to standard output — markers not used by
/// any sibling plugin. Deliberately does not sniff the spaced `::` type
/// declaration operator, since the Haskell plugin already claims that, nor
/// a bare `module `/`end` line, since the Ruby plugin already claims those;
/// placed just ahead of `text` in `CORE_PLUGINS`, no ordering constraint
/// against a specific sibling since it has no overlapping markers.
fn has_fortran_syntax(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("implicit none")
        || lower.replace(' ', "").contains("write(*,*)")
        || text.lines().any(|line| {
            starts_with_ci(line, "program ")
                || starts_with_ci(line, "subroutine ")
                || starts_with_ci(line, "end program")
                || starts_with_ci(line, "end subroutine")
                || starts_with_ci(line, "end function")
                || starts_with_ci(line, "end module")
        })
}

/// The Fortran plugin's core half.
#[derive(Debug, Default)]
pub struct FortranCore;

impl PluginCore for FortranCore {
    fn name(&self) -> &'static str {
        "fortran"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_fortran_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (programs, subroutines) = parse_definitions(&content);
        let view = FortranView {
            content,
            truncated,
            programs,
            subroutines,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Fortran plugin's presentation half.
#[derive(Debug, Default)]
pub struct FortranPresentation;

impl PluginPresentation for FortranPresentation {
    fn name(&self) -> &'static str {
        "fortran"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: FortranView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.programs.is_empty() {
            lines.push(format!("programs: {}", view.programs.join(", ")));
        }
        if !view.subroutines.is_empty() {
            lines.push(format!("subroutines: {}", view.subroutines.join(", ")));
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
    use super::{FortranCore, FortranPresentation, FortranView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-fortran-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_common_fortran_markers_as_fortran() {
        assert!(FortranCore.sniff(b"program hello\n    implicit none\nend program hello\n"));
        assert!(
            FortranCore.sniff(b"subroutine greet(n)\n    integer :: n\nend subroutine greet\n")
        );
        assert!(FortranCore.sniff(b"      write(*,*) 'hi'\n"));
        assert!(FortranCore.sniff(b"      write (*, *) 'hi'\n"));
        assert!(FortranCore.sniff(b"module shapes\nend module shapes\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_or_plain_text_as_fortran() {
        assert!(!FortranCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!FortranCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!FortranCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!FortranCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(
            !FortranCore.sniff(b"module Greeter where\n\ngreet :: String -> String\ngreet n = n\n")
        );
        assert!(!FortranCore.sniff(b"module Greeter\n  def greet\n    puts 'hi'\n  end\nend\n"));
        assert!(!FortranCore.sniff(b"just a regular line of text\n"));
        assert!(!FortranCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_fortran_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.f90");
        std::fs::write(
            &path,
            "program greeter\n    implicit none\n    call greet(1)\nend program greeter\n\nsubroutine greet(n)\n    implicit none\n    integer, intent(in) :: n\n    write(*,*) 'Hello, iteration', n\nend subroutine greet\n",
        )
        .unwrap();

        let data = FortranCore.view(&path).unwrap();
        let view: FortranView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.programs, vec!["greeter"]);
        assert_eq!(view.subroutines, vec!["greet"]);
        assert!(view.content.contains("Hello, iteration"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.f90");
        let mut content = "program pad\n".to_owned();
        content.push_str(&"!".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = FortranCore.view(&path).unwrap();
        let view: FortranView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_programs_subroutines_and_content() {
        let data = serde_json::to_value(FortranView {
            content: "program p\nend program p".to_owned(),
            truncated: false,
            programs: vec!["p".to_owned()],
            subroutines: vec!["greet".to_owned()],
        })
        .unwrap();

        let lines = FortranPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "programs: p",
                "subroutines: greet",
                "program p",
                "end program p"
            ]
        );
    }
}
