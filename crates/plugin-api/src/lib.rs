//! The core and presentation plugin traits, and the registration macro.
//!
//! A registration macro is not implemented yet: with a single plugin
//! (`plugin-text`) registered by hand in `service` and `tui`, generating the
//! static table would be structure with no second caller to justify it. Add
//! it once enough plugins exist that hand-written registration is repetitive.

pub mod thumbnail;

use std::io;
use std::path::Path;

/// The core half of a file-type plugin: sniffs and reads untrusted bytes
/// inside the service, producing view data ready for the wire.
pub trait PluginCore: Send + Sync {
    /// The file type's identifier, shared with its presentation half.
    fn name(&self) -> &'static str;

    /// Looks at a bounded prefix of a file's bytes and decides whether this
    /// plugin recognises the format.
    fn sniff(&self, prefix: &[u8]) -> bool;

    /// Reads `path` and returns its view data, ready to serialize onto the
    /// wire.
    ///
    /// # Errors
    /// Returns an error if `path` cannot be read.
    fn view(&self, path: &Path) -> io::Result<serde_json::Value>;

    /// The lowercase extensions this type claims, without their dot - the
    /// same list its presentation half reports.
    ///
    /// GUIDANCE.md §3.3 makes sniffing "content-based (magic bytes) with the
    /// extension as a hint only". For a binary format the magic bytes decide
    /// and this is never consulted. For the source languages there are no
    /// magic bytes, only keywords that genuinely overlap - `struct` belongs
    /// to C, C++, Rust, Swift and Solidity alike - and this is the hint that
    /// settles which of the matching plugins owns the file.
    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    /// The plugins this one refines, by name.
    ///
    /// A JSON Schema is JSON. An npm lock file is JSON. A Kubernetes
    /// manifest is YAML. When the general plugin and the specialisation
    /// both recognise a file, the specialisation is the better answer -
    /// but the general plugin usually owns the extension, and
    /// [`Self::extensions`] would hand the file to it on that basis alone.
    ///
    /// Saying so here settles it. The extension hint keeps the job it was
    /// added for - choosing between siblings that have no magic bytes and
    /// genuinely overlap, which is what a C file opening as Rust needed
    /// (#272) - and stops overruling a plugin strictly more specific than
    /// the one it beat.
    fn specialises(&self) -> &'static [&'static str] {
        &[]
    }
}

/// The core half of a folder plugin: decides whether a folder is a
/// programming project of some kind, and reads its manifest.
///
/// Separate from [`PluginCore`] because folders answer a different
/// question. A file has exactly one type - two plugins claiming one file
/// is a defect, which is why [`PluginCore::extensions`] exists to settle
/// it. A folder is several things at once and honestly so: this
/// repository's own root is a source control working copy *and* a Cargo
/// workspace, and a reader wants both facts. So every folder plugin that
/// recognises a folder contributes, and the pane shows the union.
///
/// Sniffing takes the names of the entries directly inside the folder
/// rather than a prefix of bytes. A folder has no bytes, and every
/// project marker there is - `Cargo.toml`, `package.json`, `go.mod`,
/// `pom.xml` - is a file name.
pub trait FolderCore: Send + Sync {
    /// The project kind's identifier, shared with its presentation half.
    fn name(&self) -> &'static str;

    /// Whether this plugin recognises a folder holding entries named
    /// `entries`, which are the names directly inside it and not a
    /// recursive walk.
    fn sniff(&self, entries: &[&str]) -> bool;

    /// Reads the folder at `path` and returns its view data, ready to
    /// serialize onto the wire.
    ///
    /// Reading means reading. Per decision D10 a folder plugin never runs
    /// the project's build tool, and never writes to the folder.
    ///
    /// # Errors
    /// Returns an error if the folder's manifest cannot be read or does
    /// not parse.
    fn view(&self, path: &Path) -> io::Result<serde_json::Value>;
}

/// The presentation half of a folder plugin: turns the core half's view
/// data into the lines a front end appends to a folder's details.
///
/// The lines are added to what the folder already reports, never
/// substituted for it. A folder that is a Cargo workspace is still a
/// folder, and a reader still wants to see what is inside it.
pub trait FolderPresentation: Send + Sync {
    /// The project kind's identifier, shared with its core half.
    fn name(&self) -> &'static str;

    /// Turns `data` (as produced by the matching core half) into lines.
    fn present(&self, data: &serde_json::Value) -> Vec<String>;
}

/// How a file type is marked in a listing: a short label and the colour it
/// is drawn in. GUIDANCE.md §3 makes the icon the plugin's own property, so
/// the plugin states these two facts and each front end draws them the way
/// its medium allows - the GUI composes a document-shaped image, a terminal
/// front end can print the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon {
    /// Up to four characters naming the type, e.g. `"RS"` or `"HTML"`.
    pub label: &'static str,
    /// The type's colour as `0xRRGGBB`.
    pub tint: u32,
}

/// The mark given to a file no plugin claims. Its label is empty, which a
/// front end should draw as a plain sheet with no type band - Explorer's
/// generic document, not an error.
pub const UNKNOWN_ICON: Icon = Icon {
    label: "",
    tint: 0x009c_a3af,
};

/// Something a front end can draw for a file whose content is a picture.
///
/// GUIDANCE.md §3 gives each plugin its own "thumbnail and graphics", but
/// the presentation half only ever returned lines of text, so no plugin
/// could show one: an image previewed as the words "Png image, 16 x 16
/// pixels". Both shapes here are toolkit-neutral - decoded pixels, or SVG
/// source a front end renders itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Graphic {
    /// Decoded pixels, row-major RGBA8, `width * height * 4` bytes long.
    Rgba {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
        /// The pixels themselves.
        pixels: Vec<u8>,
    },
    /// SVG source, for a front end that can render vectors.
    Svg(String),
}

/// The view every type offers: the plugin's own rendering of the file.
pub const PREVIEW_VIEW: &str = "Preview";

/// The view a type carrying the file's text offers alongside its preview:
/// that text, as the file holds it.
pub const TEXT_VIEW: &str = "Text";

/// The presentation half of a file-type plugin: turns the core half's view
/// data into lines of text a front end can render, without ever touching
/// raw file bytes.
pub trait PluginPresentation: Send + Sync {
    /// The file type's identifier, shared with its core half.
    fn name(&self) -> &'static str;

    /// Turns `data` (as produced by the matching core half) into the lines
    /// a front end should display.
    fn present(&self, data: &serde_json::Value) -> Vec<String>;

    /// How this type is marked in a listing.
    ///
    /// A listing has to mark hundreds of rows at once, so - as in Explorer -
    /// the mark is chosen from the file's name, never by reading it. Content
    /// sniffing stays where it belongs, in [`PluginCore::sniff`], deciding
    /// which viewer opens a file once one is chosen.
    fn icon(&self) -> Icon {
        UNKNOWN_ICON
    }

    /// The lowercase extensions this type claims, without their dot. Each
    /// extension belongs to exactly one plugin.
    fn extensions(&self) -> &'static [&'static str] {
        &[]
    }

    /// A picture to draw for this view, for the file types that are one.
    /// `None` for everything else, which is most of them: source code and
    /// structured data say more as text.
    fn graphic(&self, data: &serde_json::Value) -> Option<Graphic> {
        let _ = data;
        None
    }

    /// The views this type offers for `data`, in the order a front end
    /// should present them. Never empty: the first is what the pane shows
    /// until the reader asks for another.
    ///
    /// GUIDANCE.md §2.4 makes the File pane "supplied entirely by the
    /// file-type plugin", but a type could only ever offer one rendering,
    /// so every source-language plugin prepends its outline to the file's
    /// text: one list of lines is all it had. The default here reads the
    /// same `content` convention [`Self::editable_text`] does, so a type
    /// carrying its text also offers that text plainly, without the
    /// plugin's commentary, at no per-plugin cost. A plugin with more to
    /// show overrides both this and [`Self::present_view`].
    ///
    /// Truncation does not remove the plain view the way it removes the
    /// editor: saving part of a file back would discard the rest, but
    /// reading part of one is exactly what a long file needs.
    fn views(&self, data: &serde_json::Value) -> Vec<&'static str> {
        if data
            .get("content")
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            vec![PREVIEW_VIEW, TEXT_VIEW]
        } else {
            vec![PREVIEW_VIEW]
        }
    }

    /// Renders the view named `view`, which is one of [`Self::views`].
    ///
    /// An unrecognised name falls back to [`Self::present`] rather than
    /// erroring: a front end asking for a view this type does not offer is
    /// a front-end bug, and a preview is a better answer than a blank pane.
    fn present_view(&self, view: &str, data: &serde_json::Value) -> Vec<String> {
        if view == TEXT_VIEW
            && let Some(text) = data.get("content").and_then(serde_json::Value::as_str)
        {
            return text.lines().map(str::to_owned).collect();
        }
        self.present(data)
    }

    /// The file's text, when this type can be edited as text. `None` for a
    /// type that is not text, or for a view holding only part of one.
    ///
    /// GUIDANCE.md §3 gives every plugin a "viewer, editor"; this is the
    /// editor half's input. The default reads the convention the whole
    /// catalogue already follows on the wire - a `content` string holding
    /// the file's text, and a `truncated` flag saying whether that is all
    /// of it - so a text plugin is editable without writing anything, and a
    /// plugin whose data is shaped differently overrides.
    ///
    /// **A truncated view is never editable.** Every text plugin caps what
    /// it reads, and saving a capped view back would silently discard
    /// everything past the cap. Absent means "not truncated", since a view
    /// that never truncates has no reason to carry the flag.
    fn editable_text(&self, data: &serde_json::Value) -> Option<String> {
        let truncated = data
            .get("truncated")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if truncated {
            return None;
        }
        data.get("content")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }
}

#[cfg(test)]
mod tests {
    use super::{PREVIEW_VIEW, PluginPresentation, TEXT_VIEW};

    /// A type carrying its text on the wire, like the 56 that do.
    struct Textish;

    impl PluginPresentation for Textish {
        fn name(&self) -> &'static str {
            "textish"
        }
        fn present(&self, _data: &serde_json::Value) -> Vec<String> {
            vec!["functions: main".to_owned(), "fn main() {}".to_owned()]
        }
    }

    /// A type with nothing to read as text, like the 25 that carry none.
    struct Binaryish;

    impl PluginPresentation for Binaryish {
        fn name(&self) -> &'static str {
            "binaryish"
        }
        fn present(&self, _data: &serde_json::Value) -> Vec<String> {
            vec!["3 pages".to_owned()]
        }
    }

    /// A type that renders a view of its own rather than taking the default.
    struct Tabular;

    impl PluginPresentation for Tabular {
        fn name(&self) -> &'static str {
            "tabular"
        }
        fn present(&self, _data: &serde_json::Value) -> Vec<String> {
            vec!["2 rows".to_owned()]
        }
        fn views(&self, _data: &serde_json::Value) -> Vec<&'static str> {
            vec![PREVIEW_VIEW, "Table"]
        }
        fn present_view(&self, view: &str, data: &serde_json::Value) -> Vec<String> {
            if view == "Table" {
                return vec!["a | b".to_owned()];
            }
            self.present(data)
        }
    }

    fn with_content(content: &str, truncated: bool) -> serde_json::Value {
        serde_json::json!({ "content": content, "truncated": truncated })
    }

    #[test]
    fn a_type_carrying_text_offers_that_text_as_a_second_view() {
        let data = with_content("fn main() {}\n", false);
        assert_eq!(Textish.views(&data), vec![PREVIEW_VIEW, TEXT_VIEW]);
        assert_eq!(
            Textish.present_view(TEXT_VIEW, &data),
            vec!["fn main() {}".to_owned()],
            "the file's own text, without the outline the preview prepends"
        );
    }

    #[test]
    fn a_type_carrying_no_text_offers_one_view() {
        let data = serde_json::json!({ "pages": 3 });
        assert_eq!(Binaryish.views(&data), vec![PREVIEW_VIEW]);
    }

    #[test]
    fn a_truncated_view_still_reads_as_text_though_it_cannot_be_edited() {
        let data = with_content("first half", true);
        assert_eq!(Textish.views(&data), vec![PREVIEW_VIEW, TEXT_VIEW]);
        assert_eq!(
            Textish.present_view(TEXT_VIEW, &data),
            vec!["first half".to_owned()]
        );
        assert_eq!(
            Textish.editable_text(&data),
            None,
            "reading part of a file is fine; saving part of one back is not"
        );
    }

    #[test]
    fn the_first_view_is_the_plugins_own_rendering() {
        let data = with_content("fn main() {}", false);
        assert_eq!(
            Textish.present_view(PREVIEW_VIEW, &data),
            Textish.present(&data)
        );
    }

    #[test]
    fn a_view_this_type_does_not_offer_falls_back_to_its_preview() {
        let data = with_content("fn main() {}", false);
        assert_eq!(Textish.present_view("Hex", &data), Textish.present(&data));
    }

    #[test]
    fn a_plugin_that_overrides_is_rendered_by_its_own_implementation() {
        let data = with_content("a,b\n1,2\n", false);
        assert_eq!(Tabular.views(&data), vec![PREVIEW_VIEW, "Table"]);
        assert_eq!(
            Tabular.present_view("Table", &data),
            vec!["a | b".to_owned()],
            "the override decides, not the content convention"
        );
    }
}
