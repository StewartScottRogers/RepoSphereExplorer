//! Web application manifest file type plugin: core and presentation halves.
//!
//! A specialisation of JSON: `name`, `start_url` and `display` together
//! are a shape no other document has.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["webmanifest"];

/// Maximum number of bytes read from a file when viewing it.
const MAX_VIEW_BYTES: usize = 64 * 1024;

/// One declared icon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconEntry {
    /// Where the image lives.
    pub src: String,
    /// The sizes it declares, as written.
    pub sizes: String,
    /// Its media type, when stated.
    pub kind: Option<String>,
    /// Its purpose, such as `maskable`.
    pub purpose: Option<String>,
}

/// View data produced by [`WebmanifestCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebmanifestView {
    /// The application's name.
    pub name: Option<String>,
    /// The short name a launcher uses when space is tight.
    pub short_name: Option<String>,
    /// The address it opens at.
    pub start_url: Option<String>,
    /// How it is displayed: `standalone`, `fullscreen`, `browser`.
    pub display: Option<String>,
    /// The theme colour, which paints the surrounding chrome.
    pub theme_color: Option<String>,
    /// The background colour, shown while it loads.
    pub background_color: Option<String>,
    /// The declared icons.
    pub icons: Vec<IconEntry>,
    /// The shortcut names.
    pub shortcuts: Vec<String>,
    /// The keys this reader does not know, so nothing is silently dropped.
    pub other_keys: Vec<String>,
    /// Whether `content` was cut off at [`MAX_VIEW_BYTES`].
    pub truncated: bool,
}

/// The keys this reader understands.
const KNOWN: &[&str] = &[
    "name",
    "short_name",
    "start_url",
    "display",
    "theme_color",
    "background_color",
    "icons",
    "shortcuts",
];

/// Everything [`WebmanifestView`] holds, read from `text`.
fn parse(text: &str) -> WebmanifestView {
    let mut view = WebmanifestView {
        name: None,
        short_name: None,
        start_url: None,
        display: None,
        theme_color: None,
        background_color: None,
        icons: Vec::new(),
        shortcuts: Vec::new(),
        other_keys: Vec::new(),
        truncated: false,
    };
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return view;
    };
    let text_at = |key: &str| root.get(key).and_then(Value::as_str).map(str::to_owned);

    view.name = text_at("name");
    view.short_name = text_at("short_name");
    view.start_url = text_at("start_url");
    view.display = text_at("display");
    view.theme_color = text_at("theme_color");
    view.background_color = text_at("background_color");

    if let Some(icons) = root.get("icons").and_then(Value::as_array) {
        for icon in icons {
            view.icons.push(IconEntry {
                src: icon
                    .get("src")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                sizes: icon
                    .get("sizes")
                    .and_then(Value::as_str)
                    .unwrap_or("unstated")
                    .to_owned(),
                kind: icon.get("type").and_then(Value::as_str).map(str::to_owned),
                purpose: icon
                    .get("purpose")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }
    if let Some(shortcuts) = root.get("shortcuts").and_then(Value::as_array) {
        for shortcut in shortcuts {
            if let Some(name) = shortcut.get("name").and_then(Value::as_str) {
                view.shortcuts.push(name.to_owned());
            }
        }
    }
    if let Some(object) = root.as_object() {
        for key in object.keys() {
            if !KNOWN.contains(&key.as_str()) {
                view.other_keys.push(key.clone());
            }
        }
    }
    view
}

/// Whether `text` is a web application manifest.
fn looks_like_it(text: &str) -> bool {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return false;
    };
    let has = |key: &str| root.get(key).is_some();
    // `start_url` is the one key nothing else has; `name` and `display`
    // alongside it settle any doubt.
    has("start_url") && (has("name") || has("short_name")) && has("display")
}

/// The Web application manifest plugin's core half.
#[derive(Debug, Default)]
pub struct WebmanifestCore;

impl PluginCore for WebmanifestCore {
    fn name(&self) -> &'static str {
        "webmanifest"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn specialises(&self) -> &'static [&'static str] {
        // A specialisation of JSON, which owns the extension. Without this
        // the extension hint hands the file over whatever the order (D13).
        &["json"]
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        std::str::from_utf8(prefix).is_ok_and(looks_like_it)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let bytes = std::fs::read(path)?;
        let truncated = bytes.len() > MAX_VIEW_BYTES;
        let slice = &bytes[..bytes.len().min(MAX_VIEW_BYTES)];
        // The manifest is small, but every field worth reading is already
        // named on the view, so the raw text would only repeat it.
        let mut view = parse(&String::from_utf8_lossy(slice));
        view.truncated = truncated;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The Web application manifest plugin's presentation half.
#[derive(Debug, Default)]
pub struct WebmanifestPresentation;

impl PluginPresentation for WebmanifestPresentation {
    fn name(&self) -> &'static str {
        "webmanifest"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PWA",
            tint: 0x005a_0fc0,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: WebmanifestView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = Vec::new();
        if let Some(name) = &view.name {
            let short = view
                .short_name
                .as_ref()
                .map_or_else(String::new, |short| format!(" ({short})"));
            lines.push(format!("{name}{short}"));
        }
        if let Some(start) = &view.start_url {
            lines.push(format!("Opens at: {start}"));
        }
        if let Some(display) = &view.display {
            lines.push(format!("Display: {display}"));
        }
        match (&view.theme_color, &view.background_color) {
            (Some(theme), Some(background)) => {
                lines.push(format!("Colours: theme {theme}, background {background}"));
            }
            (Some(theme), None) => lines.push(format!("Theme colour: {theme}")),
            (None, Some(background)) => lines.push(format!("Background colour: {background}")),
            (None, None) => {}
        }
        if !view.icons.is_empty() {
            lines.push(format!("Icons ({}):", view.icons.len()));
            for icon in &view.icons {
                let purpose = icon
                    .purpose
                    .as_ref()
                    .map_or_else(String::new, |purpose| format!(", {purpose}"));
                lines.push(format!("  {}  {}{purpose}", icon.sizes, icon.src));
            }
        }
        if !view.shortcuts.is_empty() {
            lines.push(format!("Shortcuts: {}", view.shortcuts.join(", ")));
        }
        if !view.other_keys.is_empty() {
            lines.push(format!("Also set: {}", view.other_keys.join(", ")));
        }

        if view.truncated {
            lines.push("… (truncated)".to_owned());
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{WebmanifestCore, WebmanifestPresentation, WebmanifestView, parse};
    use plugin_api::{PluginCore, PluginPresentation};

    const MANIFEST: &str = r##"{
      "name": "Repos Explorer",
      "short_name": "Repos",
      "start_url": "/?source=pwa",
      "display": "standalone",
      "theme_color": "#1f2933",
      "background_color": "#ffffff",
      "icons": [
        { "src": "/icons/192.png", "sizes": "192x192", "type": "image/png" },
        { "src": "/icons/512.png", "sizes": "512x512", "type": "image/png",
          "purpose": "maskable" }
      ],
      "shortcuts": [ { "name": "Open the Repos Directory", "url": "/roots" } ],
      "orientation": "portrait"
    }"##;

    #[test]
    fn sniffs_the_three_keys_together() {
        assert!(WebmanifestCore.sniff(MANIFEST.as_bytes()));
    }

    #[test]
    fn does_not_claim_json_that_merely_has_a_name() {
        assert!(!WebmanifestCore.sniff(br#"{"name": "a", "version": "1"}"#));
        assert!(!WebmanifestCore.sniff(br#"{"start_url": "/"}"#));
        assert!(!WebmanifestCore.sniff(b""));
    }

    #[test]
    fn it_says_it_specialises_json() {
        assert_eq!(WebmanifestCore.specialises(), &["json"]);
    }

    #[test]
    fn reads_the_identity_and_the_colours() {
        let view = parse(MANIFEST);

        assert_eq!(view.name.as_deref(), Some("Repos Explorer"));
        assert_eq!(view.short_name.as_deref(), Some("Repos"));
        assert_eq!(view.display.as_deref(), Some("standalone"));
        assert_eq!(view.theme_color.as_deref(), Some("#1f2933"));
    }

    #[test]
    fn reads_every_icon_with_its_purpose() {
        let view = parse(MANIFEST);

        assert_eq!(view.icons.len(), 2);
        assert_eq!(view.icons[0].sizes, "192x192");
        assert_eq!(view.icons[1].purpose.as_deref(), Some("maskable"));
    }

    #[test]
    fn reports_keys_it_does_not_know_rather_than_dropping_them() {
        let view = parse(MANIFEST);

        assert_eq!(view.other_keys, vec!["orientation".to_owned()]);
    }

    #[test]
    fn an_icon_with_no_sizes_says_so_rather_than_showing_nothing() {
        let view = parse(
            r#"{"name":"a","start_url":"/","display":"browser",
                             "icons":[{"src":"/a.png"}]}"#,
        );

        assert_eq!(view.icons[0].sizes, "unstated");
    }

    #[test]
    fn presents_the_name_first() {
        let data = serde_json::to_value(parse(MANIFEST)).unwrap();

        let lines = WebmanifestPresentation.present(&data);

        assert_eq!(lines[0], "Repos Explorer (Repos)");
    }

    #[test]
    fn the_repository_fixture_fills_every_field() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/webmanifest/app.webmanifest");

        let data = WebmanifestCore.view(&path).unwrap();
        let view: WebmanifestView = serde_json::from_value(data).unwrap();

        assert!(view.name.is_some() && view.short_name.is_some());
        assert!(view.start_url.is_some() && view.display.is_some());
        assert!(view.theme_color.is_some() && view.background_color.is_some());
        assert!(view.icons.len() >= 3);
        assert!(view.icons.iter().any(|icon| icon.purpose.is_some()));
        assert!(!view.shortcuts.is_empty());
        assert!(!view.other_keys.is_empty());
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            plugin_api::PluginCore::extensions(&crate::WebmanifestCore),
            plugin_api::PluginPresentation::extensions(&crate::WebmanifestPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }
}
