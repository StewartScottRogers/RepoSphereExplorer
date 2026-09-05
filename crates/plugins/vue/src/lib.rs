//! Vue single-file component file type plugin: core and presentation halves.

use plugin_api::{PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// View data produced by [`VueCore::view`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VueView {
    /// The file's content, decoded as UTF-8 (lossily, if necessary).
    pub content: String,
    /// Whether the content was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
    /// Top-level block-opening tags found (e.g. `template`, `script setup`,
    /// `style scoped`), in source order.
    pub sections: Vec<String>,
}

/// Whether `line` opens a top-level Vue SFC block (`<template`, `<script`,
/// or `<style`, each conventionally written at column zero).
fn is_section_opener(line: &str) -> bool {
    line.starts_with("<template") || line.starts_with("<script") || line.starts_with("<style")
}

/// Parses top-level section names (a tag's name plus any attributes) out of
/// `content`, in source order, e.g. `<script setup>` becomes `script setup`.
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

/// Whether `text` looks like a Vue single-file component: a top-level
/// `<template>` block paired with a top-level `<script>` block. Neither tag
/// alone is unique to Vue (native HTML5 also defines `<template>`, and
/// `<script>` is ordinary HTML), but this project's `html` plugin sniffs by
/// document-structure tags (`<!doctype html`, `<html`, `<head>`, `<body>`,
/// `<title>`) that a Vue SFC does not carry, so the combination is not
/// claimed by it first.
fn has_vue_sfc_syntax(text: &str) -> bool {
    let has_template = text.contains("<template>") || text.contains("<template ");
    let has_script = text.contains("<script>") || text.contains("<script ");
    has_template && has_script
}

/// The Vue plugin's core half.
#[derive(Debug, Default)]
pub struct VueCore;

impl PluginCore for VueCore {
    fn name(&self) -> &'static str {
        "vue"
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(prefix) else {
            return false;
        };
        has_vue_sfc_syntax(text)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        let content = String::from_utf8_lossy(slice).into_owned();
        let sections = parse_sections(&content);
        let view = VueView {
            content,
            truncated,
            sections,
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Vue plugin's presentation half.
#[derive(Debug, Default)]
pub struct VuePresentation;

impl PluginPresentation for VuePresentation {
    fn name(&self) -> &'static str {
        "vue"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let view: VueView = match serde_json::from_value(data.clone()) {
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
    use super::{MAX_VIEW_BYTES, VueCore, VuePresentation, VueView};
    use plugin_api::{PluginCore, PluginPresentation};

    fn unique_temp_file(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "rse-plugin-vue-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn sniffs_a_template_and_script_pair_as_vue() {
        assert!(VueCore.sniff(
            b"<template>\n  <div>{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\n</script>\n"
        ));
        assert!(VueCore.sniff(
            b"<template>\n  <p>hi</p>\n</template>\n<script>\nexport default {}\n</script>\n"
        ));
    }

    #[test]
    fn does_not_sniff_plain_html_or_a_bare_template_tag_as_vue() {
        assert!(!VueCore.sniff(
            b"<!doctype html>\n<html>\n<head><title>hi</title></head>\n<body></body>\n</html>\n"
        ));
        assert!(!VueCore.sniff(b"<template>\n  <p>only a template, no script</p>\n</template>\n"));
        assert!(!VueCore.sniff(b"<script>\nconsole.log('no template here');\n</script>\n"));
        assert!(!VueCore.sniff(b"just a regular line of text\n"));
        assert!(!VueCore.sniff(&[0xFF, 0xFE, 0x00, 0x00]));
    }

    #[test]
    fn views_a_real_vue_sfc_and_extracts_sections() {
        let path = unique_temp_file("Widget.vue");
        std::fs::write(
            &path,
            "<template>\n  <div class=\"widget\">{{ msg }}</div>\n</template>\n\n<script setup>\nimport { ref } from 'vue'\nconst msg = ref('hi')\n</script>\n\n<style scoped>\n.widget { color: red; }\n</style>\n",
        )
        .unwrap();

        let data = VueCore.view(&path).unwrap();
        let view: VueView = serde_json::from_value(data).unwrap();

        assert!(!view.truncated);
        assert_eq!(
            view.sections,
            vec!["template", "script setup", "style scoped"]
        );
        assert!(view.content.contains("const msg = ref"));

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn truncates_a_file_larger_than_the_view_limit() {
        let path = unique_temp_file("large.vue");
        let mut content = "<template>\n".to_owned();
        content.push_str(&"  <!-- -->\n".repeat(MAX_VIEW_BYTES + 10));
        content.push_str("</template>\n<script>\nexport default {}\n</script>\n");
        std::fs::write(&path, content).unwrap();

        let data = VueCore.view(&path).unwrap();
        let view: VueView = serde_json::from_value(data).unwrap();

        assert_eq!(view.content.len(), MAX_VIEW_BYTES);
        assert!(view.truncated);

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn presents_sections_and_content() {
        let data = serde_json::to_value(VueView {
            content: "<template>\n  <p>hi</p>\n</template>".to_owned(),
            truncated: false,
            sections: vec!["template".to_owned()],
        })
        .unwrap();

        let lines = VuePresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "sections: template",
                "<template>",
                "  <p>hi</p>",
                "</template>"
            ]
        );
    }
}
