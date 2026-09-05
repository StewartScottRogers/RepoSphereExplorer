//! Svelte component file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// Svelte-only template syntax markers, checked anywhere in the content —
/// markers not used by any sibling plugin. A Svelte component has no
/// wrapping `<template>` tag the way a Vue SFC does (see `plugin-vue`), so
/// this plugin instead sniffs the block/expression syntax unique to
/// Svelte's own template language.
const SVELTE_MARKERS: &[&str] = &[
    "{#if ",
    "{#each ",
    "{#await ",
    "{@html ",
    "{@debug ",
    "{:else",
    "{:then",
    "{:catch",
    "<script context=\"module\">",
];

/// View data produced by [`SvelteCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SvelteView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Top-level block-opening tags found (e.g. `script`,
    /// `script context="module"`, `style`), in source order.
    pub sections: Vec<String>,
}

/// Whether `line` opens a top-level Svelte SFC block (`<script` or
/// `<style`, each conventionally written at column zero). Unlike a Vue SFC,
/// a Svelte component has no `<template>` wrapper: its markup lives at the
/// top level, alongside these two optional blocks.
fn is_section_opener(line: &str) -> bool {
    line.starts_with("<script") || line.starts_with("<style")
}

/// Parses top-level section names (a tag's name plus any attributes) out of
/// `content`, in source order, e.g. `<script context="module">` becomes
/// `script context="module"`.
fn parse_sections(content: &str) -> Vec<String> {
    content
        .lines()
        .filter(|line| is_section_opener(line))
        .filter_map(|line| {
            let inner = line.strip_prefix('<')?;
            let end = inner.find('>')?;
            Some(inner[..end].trim().to_owned())
        })
        .collect()
}

/// Whether `text` looks like a Svelte component: it carries one of
/// [`SVELTE_MARKERS`], or a top-level `$:` reactive statement (checked line
/// by line, since a bare `$:` substring is otherwise too easy to false
/// positive on).
fn has_svelte_syntax(text: &str) -> bool {
    SVELTE_MARKERS.iter().any(|marker| text.contains(marker))
        || text.lines().any(|line| line.trim_start().starts_with("$:"))
}

/// The Svelte plugin's core half.
#[derive(Debug, Default)]
pub struct SvelteCore;

impl PluginCore for SvelteCore {
    fn name(&self) -> &'static str {
        "svelte"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_svelte_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let sections = parse_sections(&content);
        let view = SvelteView {
            content,
            truncated,
            sections,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Svelte plugin's presentation half.
#[derive(Debug, Default)]
pub struct SveltePresentation;

impl PluginPresentation for SveltePresentation {
    fn name(&self) -> &'static str {
        "svelte"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: SvelteView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if !view.sections.is_empty() {
            lines.push(format!("sections: {}", view.sections.join(", ")));
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
    use super::{MAX_VIEW_BYTES, SvelteCore, SveltePresentation, SvelteView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-svelte-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_svelte_block_syntax() {
        assert!(SvelteCore.sniff(b"<script>\n  let count = 0;\n</script>\n\n<button>{count}</button>\n\n{#if count > 0}\n  <p>positive</p>\n{/if}\n"));
        assert!(SvelteCore.sniff(b"{#each items as item}\n  <li>{item}</li>\n{/each}\n"));
        assert!(
            SvelteCore.sniff(b"{#await promise}\n  loading\n{:then value}\n  {value}\n{/await}\n")
        );
        assert!(SvelteCore.sniff(b"<div>{@html rawHtml}</div>\n"));
        assert!(
            SvelteCore.sniff(
                b"<script context=\"module\">\n  export const load = () => ({});\n</script>\n"
            )
        );
        assert!(
            SvelteCore.sniff(b"<script>\n  let count = 0;\n  $: doubled = count * 2;\n</script>\n")
        );
    }

    #[test]
    fn does_not_sniff_plain_html_or_a_bare_script_tag_as_svelte() {
        assert!(!SvelteCore.sniff(
            b"<!doctype html>\n<html>\n<head><title>hi</title></head>\n<body></body>\n</html>\n"
        ));
        assert!(!SvelteCore.sniff(b"<script>\nconsole.log('no svelte syntax here');\n</script>\n"));
        assert!(!SvelteCore.sniff(
            b"<template>\n  <div>{{ msg }}</div>\n</template>\n<script>\nexport default {}\n</script>\n"
        ));
        assert!(!SvelteCore.sniff(b"just a regular line of text\n"));
        assert!(!SvelteCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_svelte_component_and_extracts_sections() {
        let path = unique_temp_file("Widget.svelte");
        std::fs::write(
            &path,
            "<script>\n  export let name;\n</script>\n\n<h1>Hello {name}!</h1>\n\n{#if name}\n  <p>known</p>\n{/if}\n\n<style>\n  h1 { color: red; }\n</style>\n",
        )
        .unwrap();

        let data = SvelteCore.view(&path).unwrap();
        let view: SvelteView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(view.sections, vec!["script", "style"]);
        assert!(view.content.contains("export let name"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.svelte");
        let mut content = "{#each items as item}\n".to_owned();
        content.push_str(&"  <li>{item}</li>\n".repeat(MAX_VIEW_BYTES + 10));
        content.push_str("{/each}\n");
        std::fs::write(&path, content).unwrap();

        let data = SvelteCore.view(&path).unwrap();
        let view: SvelteView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_sections_and_content() {
        let data = serde_json::to_value(SvelteView {
            content: "<script>\n  let x = 1;\n</script>".to_owned(),
            truncated: false,
            sections: vec!["script".to_owned()],
        })
        .unwrap();

        let lines = SveltePresentation.present(&data);

        assert_eq!(
            lines,
            vec!["sections: script", "<script>", "  let x = 1;", "</script>"]
        );
    }
}
