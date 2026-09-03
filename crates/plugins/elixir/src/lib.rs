//! Elixir file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`ElixirCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElixirView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Names of top-level `def`/`defp` function declarations found in the
    /// content.
    pub functions: Vec<String>,
    /// Names of top-level `defmodule` declarations found in the content.
    pub modules: Vec<String>,
}

/// Extracts the identifier following a `def `/`defp ` keyword at the start
/// of `line` (after leading whitespace), e.g. `def greet(name) do` returns
/// `Some("greet")`. Elixir function names may end in `?` or `!`, which are
/// included in the match.
fn parse_function_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed
        .strip_prefix("defp ")
        .or_else(|| trimmed.strip_prefix("def "))?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '?' || ch == '!'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Extracts the identifier following a `defmodule ` keyword at the start of
/// `line` (after leading whitespace), e.g. `defmodule MyApp.Greeter do`
/// returns `Some("MyApp.Greeter")`.
fn parse_module_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("defmodule ")?;
    let end = rest
        .find(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '.'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parses top-level module and function names out of `content`, in source
/// order.
fn parse_definitions(content: &str) -> (Vec<String>, Vec<String>) {
    let mut functions = Vec::new();
    let mut modules = Vec::new();
    for line in content.lines() {
        if let Some(name) = parse_module_name(line) {
            modules.push(name.to_owned());
        } else if let Some(name) = parse_function_name(line) {
            functions.push(name.to_owned());
        }
    }
    (functions, modules)
}

/// Whether `text`'s first line is a shebang naming the `elixir` interpreter.
fn has_elixir_shebang(text: &str) -> bool {
    text.lines()
        .next()
        .is_some_and(|line| line.starts_with("#!") && line.contains("elixir"))
}

/// Whether `text` looks like Elixir source: markers not used by this
/// project's other source-language plugins. `defmodule ` is Elixir's module
/// declaration; `IO.puts`/`IO.inspect` are Elixir's console functions,
/// distinct from the Ruby plugin's bare `puts` check; `@moduledoc` is
/// Elixir's module-documentation attribute; and `|>` is Elixir's pipe
/// operator, distinct from the R plugin's `%>%` pipe. None of these appear
/// in a bare `end`-closed block on their own, which is why this plugin does
/// not rely on that marker (the Ruby plugin already claims bare `end`
/// lines), and is instead placed ahead of `ruby` in `CORE_PLUGINS` so a
/// genuine Elixir file's `end` lines are reached only after one of these
/// stronger, Elixir-only markers has already matched.
fn has_elixir_syntax(text: &str) -> bool {
    text.contains("defmodule ")
        || text.contains("IO.puts")
        || text.contains("IO.inspect")
        || text.contains("@moduledoc")
        || text.contains("|>")
}

/// The Elixir plugin's core half.
#[derive(Debug, Default)]
pub struct ElixirCore;

impl PluginCore for ElixirCore {
    fn name(&self) -> &'static str {
        "elixir"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_elixir_shebang(text) || has_elixir_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let (functions, modules) = parse_definitions(&content);
        let view = ElixirView {
            content,
            truncated,
            functions,
            modules,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Elixir plugin's presentation half.
#[derive(Debug, Default)]
pub struct ElixirPresentation;

impl PluginPresentation for ElixirPresentation {
    fn name(&self) -> &'static str {
        "elixir"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: ElixirView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.modules.is_empty() {
            lines.push(format!("modules: {}", view.modules.join(", ")));
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
    use super::{ElixirCore, ElixirPresentation, ElixirView, MAX_VIEW_BYTES};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-elixir-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_an_elixir_shebang_line_as_elixir() {
        assert!(ElixirCore.sniff(b"#!/usr/bin/env elixir\nIO.puts(\"hi\")\n"));
    }

    #[test]
    fn sniffs_common_elixir_markers_as_elixir() {
        assert!(ElixirCore.sniff(b"defmodule Greeter do\n  def hi, do: :ok\nend\n"));
        assert!(ElixirCore.sniff(b"IO.puts(\"hello\")\n"));
        assert!(ElixirCore.sniff(b"IO.inspect(x)\n"));
        assert!(ElixirCore.sniff(b"@moduledoc \"\"\"\nA greeter.\n\"\"\"\n"));
        assert!(ElixirCore.sniff(b"[1, 2, 3] |> Enum.map(&(&1 * 2))\n"));
    }

    #[test]
    fn does_not_sniff_other_languages_with_overlapping_syntax_as_elixir() {
        assert!(!ElixirCore.sniff(b"def greet():\n    return 1\n"));
        assert!(!ElixirCore.sniff(b"class Greeter:\n    pass\n"));
        assert!(!ElixirCore.sniff(b"def greet\n  puts 'hi'\nend\n"));
        assert!(!ElixirCore.sniff(b"module Greeter\n  def greet\n    puts 'hi'\n  end\nend\n"));
        assert!(!ElixirCore.sniff(b"<?php\necho 'hi';\n"));
        assert!(!ElixirCore.sniff(b"function greet() {\n  return 1;\n}\n"));
        assert!(!ElixirCore.sniff(b"pub fn greet() -> String {\n  String::new()\n}\n"));
        assert!(!ElixirCore.sniff(b"package main\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n"));
        assert!(!ElixirCore.sniff(
            b"#include <stdio.h>\n\nint main(void) {\n    printf(\"hi\");\n    return 0;\n}\n"
        ));
        assert!(!ElixirCore.sniff(
            b"import java.util.List;\n\npublic class Main {\n    public static void main(String[] args) {\n        System.out.println(\"hi\");\n    }\n}\n"
        ));
        assert!(!ElixirCore.sniff(b"x <- 5\nresult <- data %>% filter(x > 1)\n"));
        assert!(!ElixirCore.sniff(b"import scala.collection.mutable.ListBuffer\n"));
        assert!(!ElixirCore.sniff(b"just a regular line of text\n"));
        assert!(!ElixirCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_elixir_file_and_extracts_definitions() {
        let path = unique_temp_file("greeter.ex");
        std::fs::write(
            &path,
            "defmodule MyApp.Greeter do\n  @moduledoc \"A friendly greeter.\"\n\n  def greet(name) do\n    IO.puts(\"Hello, #{name}!\")\n  end\n\n  defp shout(name), do: String.upcase(name)\nend\n",
        )
        .unwrap();

        let data = ElixirCore.view(&path).unwrap();
        let view: ElixirView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.modules, vec!["MyApp.Greeter"]);
        assert_eq!(view.functions, vec!["greet", "shout"]);
        assert!(view.content.contains("Hello"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ex");
        let mut content = "defmodule Large do\n".to_owned();
        content.push_str(&"# ".repeat(MAX_VIEW_BYTES + 10));
        std::fs::write(&path, content).unwrap();

        let data = ElixirCore.view(&path).unwrap();
        let view: ElixirView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_modules_functions_and_content() {
        let data = serde_json::to_value(ElixirView {
            content: "defmodule A do\nend".to_owned(),
            truncated: false,
            functions: vec!["greet".to_owned()],
            modules: vec!["A".to_owned()],
        })
        .unwrap();

        let lines = ElixirPresentation.present(&data);

        assert_eq!(
            lines,
            vec!["modules: A", "functions: greet", "defmodule A do", "end"]
        );
    }
}
