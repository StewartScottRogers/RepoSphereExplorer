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
