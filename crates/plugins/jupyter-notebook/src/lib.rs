//! Jupyter Notebook file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// UTF-8 byte order mark, stripped before sniffing.
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// One notebook cell: its type, source text, and (for a code cell) its
/// rendered outputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookCell {
    /// `code`, `markdown`, `raw`, or whatever else a future nbformat
    /// version introduces.
    pub cell_type: String,
    /// The cell's source, joined from nbformat's line-array or single-string
    /// form into one string.
    pub source: String,
    /// The cell's outputs, each already rendered to display text; empty for
    /// a non-code cell.
    pub outputs: Vec<String>,
}

/// View data produced by [`NotebookCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotebookView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary), shown
    /// as a fallback when `parsed` is `None`.
    pub content: String,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// The file parsed as a notebook's cells, or `None` if it doesn't parse
    /// as one.
    pub parsed: Option<Vec<NotebookCell>>,
}

/// Strips a leading UTF-8 BOM and ASCII whitespace from `prefix`.
fn trim_prefix(prefix: &[u8]) -> &[u8] {
    let without_bom = prefix.strip_prefix(UTF8_BOM).unwrap_or(prefix);
    let start = without_bom
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(without_bom.len());
    &without_bom[start..]
}

/// Whether `prefix` looks like a Jupyter Notebook: a JSON object carrying
/// both a `cells` key and either an `nbformat` or a `cell_type` key —
/// markers not used by any sibling plugin, and specific enough that they
/// don't fire on an arbitrary JSON document. A notebook file is itself
/// valid JSON, so this plugin must be placed ahead of `json` in
/// `CORE_PLUGINS`, where its own stronger markers claim the file first.
fn looks_like_notebook(prefix: &[u8]) -> bool {
    let trimmed = trim_prefix(prefix);
    if !trimmed.starts_with(b"{") {
        return false;
    }
    let text = String::from_utf8_lossy(trimmed);
    text.contains("\"cells\"") && (text.contains("\"nbformat\"") || text.contains("\"cell_type\""))
}

/// Joins nbformat's `source` field, which is either a single string or an
/// array of line strings, into one string.
fn join_source(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Renders one entry of a code cell's `outputs` array to display text, or
/// `None` if it doesn't match a known `output_type`.
fn render_output(output: &Value) -> Option<String> {
    let output_type = output.get("output_type")?.as_str()?;
    match output_type {
        "stream" => {
            let name = output
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("stdout");
            let text = join_source(output.get("text").unwrap_or(&Value::Null));
            Some(format!("[{name}] {text}"))
        }
        "execute_result" | "display_data" => {
            let data = output.get("data")?;
            if let Some(text) = data.get("text/plain") {
                Some(format!("[{output_type}] {}", join_source(text)))
            } else {
                let mime = data
                    .as_object()
                    .and_then(|entries| entries.keys().next())
                    .map_or("unknown", String::as_str);
                Some(format!("[{output_type}] <{mime} output>"))
            }
        }
        "error" => {
            let ename = output.get("ename").and_then(Value::as_str).unwrap_or("");
            let evalue = output.get("evalue").and_then(Value::as_str).unwrap_or("");
            Some(format!("[error] {ename}: {evalue}"))
        }
        _ => None,
    }
}

/// Parses `bytes` as a notebook, returning its cells in file order, or
/// `None` if it isn't valid JSON or has no top-level `cells` array.
fn parse_notebook(bytes: &[u8]) -> Option<Vec<NotebookCell>> {
    let root: Value = serde_json::from_slice(bytes).ok()?;
    let cells = root.get("cells")?.as_array()?;
    Some(
        cells
            .iter()
            .map(|cell| {
                let cell_type = cell
                    .get("cell_type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let source = join_source(cell.get("source").unwrap_or(&Value::Null));
                let outputs = cell
                    .get("outputs")
                    .and_then(Value::as_array)
                    .map(|outputs| outputs.iter().filter_map(render_output).collect())
                    .unwrap_or_default();
                NotebookCell {
                    cell_type,
                    source,
                    outputs,
                }
            })
            .collect(),
    )
}

/// Renders `cells` as one block per cell: a `[N] <type> cell` header, its
/// source lines, and (if present) its rendered output lines.
fn present_cells(cells: &[NotebookCell]) -> Vec<String> {
    let mut lines = Vec::new();
    for (index, cell) in cells.iter().enumerate() {
        lines.push(format!("[{}] {} cell", index + 1, cell.cell_type));
        lines.extend(cell.source.lines().map(str::to_owned));
        if !cell.outputs.is_empty() {
            lines.push("output:".to_owned());
            for output in &cell.outputs {
                lines.extend(output.lines().map(str::to_owned));
            }
        }
        lines.push(String::new());
    }
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// The Jupyter Notebook plugin's core half.
#[derive(Debug, Default)]
pub struct NotebookCore;

impl PluginCore for NotebookCore {
    fn name(&self) -> &'static str {
        "jupyter-notebook"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_notebook(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let parsed = parse_notebook(&bytes);
        let view = NotebookView {
            content,
            truncated,
            parsed,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Jupyter Notebook plugin's presentation half.
#[derive(Debug, Default)]
pub struct NotebookPresentation;

impl PluginPresentation for NotebookPresentation {
    fn name(&self) -> &'static str {
        "jupyter-notebook"
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: NotebookView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = if let Some(cells) = &view.parsed {
            present_cells(cells)
        } else {
            let mut lines =
                vec!["could not parse as a Jupyter notebook; showing raw content".to_owned()];
            lines.extend(view.content.lines().map(str::to_owned));
            lines
        };
        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VIEW_BYTES, NotebookCell, NotebookCore, NotebookPresentation, NotebookView};
    use plugin_api::{PluginCore, PluginPresentation};
    use serde_json::json;

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-jupyter-notebook-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn sample_notebook() -> String {
        json!({
            "cells": [
                {
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": ["# Title\n", "Some text."]
                },
                {
                    "cell_type": "code",
                    "execution_count": 1,
                    "metadata": {},
                    "outputs": [
                        {
                            "output_type": "stream",
                            "name": "stdout",
                            "text": ["hello\n"]
                        },
                        {
                            "output_type": "execute_result",
                            "execution_count": 1,
                            "data": {"text/plain": ["2"]},
                            "metadata": {}
                        }
                    ],
                    "source": "1 + 1"
                }
            ],
            "metadata": {"kernelspec": {"name": "python3"}},
            "nbformat": 4,
            "nbformat_minor": 5
        })
        .to_string()
    }

    #[test]
    fn sniffs_a_real_notebook_as_a_notebook() {
        assert!(NotebookCore.sniff(sample_notebook().as_bytes()));
    }

    #[test]
    fn does_not_sniff_plain_json_shell_scripts_or_text_as_a_notebook() {
        assert!(!NotebookCore.sniff(b"{\"a\": 1}"));
        assert!(!NotebookCore.sniff(b"{\"cells\": \"not a notebook field\"}"));
        assert!(!NotebookCore.sniff(b"just a regular line of text\n"));
        assert!(!NotebookCore.sniff(b""));
        assert!(!NotebookCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_notebook_and_parses_its_cells() {
        let path = unique_temp_file("doc.ipynb");
        std::fs::write(&path, sample_notebook()).unwrap();

        let data = NotebookCore.view(&path).unwrap();
        let view: NotebookView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        let cells = view.parsed.expect("notebook should parse");
        assert_eq!(
            cells,
            vec![
                NotebookCell {
                    cell_type: "markdown".to_owned(),
                    source: "# Title\nSome text.".to_owned(),
                    outputs: vec![],
                },
                NotebookCell {
                    cell_type: "code".to_owned(),
                    source: "1 + 1".to_owned(),
                    outputs: vec![
                        "[stdout] hello\n".to_owned(),
                        "[execute_result] 2".to_owned(),
                    ],
                },
            ]
        );

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_file_with_no_cells_array_with_no_parsed_value() {
        let path = unique_temp_file("invalid.ipynb");
        std::fs::write(&path, "{ \"not\": \"a notebook\" }").unwrap();

        let data = NotebookCore.view(&path).unwrap();
        let view: NotebookView = serde_json::from_value(data).unwrap();

        assert_eq!(view.parsed, None);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_raw_fallback_content_of_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.ipynb");
        let mut content = "{ \"not\": \"a notebook\", \"pad\": \"".to_owned();
        while content.len() <= MAX_VIEW_BYTES {
            content.push('x');
        }
        content.push_str("\" }");
        std::fs::write(&path, &content).unwrap();

        let data = NotebookCore.view(&path).unwrap();
        let view: NotebookView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_cells_with_headers_source_and_output() {
        let data = serde_json::to_value(NotebookView {
            content: String::new(),
            truncated: false,
            parsed: Some(vec![
                NotebookCell {
                    cell_type: "markdown".to_owned(),
                    source: "# Title".to_owned(),
                    outputs: vec![],
                },
                NotebookCell {
                    cell_type: "code".to_owned(),
                    source: "1 + 1".to_owned(),
                    outputs: vec!["[execute_result] 2".to_owned()],
                },
            ]),
        })
        .unwrap();

        let lines = NotebookPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "[1] markdown cell",
                "# Title",
                "",
                "[2] code cell",
                "1 + 1",
                "output:",
                "[execute_result] 2",
            ]
        );
    }

    #[test]
    fn presents_raw_content_when_not_parseable() {
        let data = serde_json::to_value(NotebookView {
            content: "{ \"not\": \"a notebook\" }".to_owned(),
            truncated: true,
            parsed: None,
        })
        .unwrap();

        let lines = NotebookPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "could not parse as a Jupyter notebook; showing raw content",
                "{ \"not\": \"a notebook\" }",
                "… (truncated)",
            ]
        );
    }
}
