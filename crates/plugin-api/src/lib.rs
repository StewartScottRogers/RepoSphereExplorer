//! The core and presentation plugin traits, and the registration macro.
//!
//! A registration macro is not implemented yet: with a single plugin
//! (`plugin-text`) registered by hand in `service` and `tui`, generating the
//! static table would be structure with no second caller to justify it. Add
//! it once enough plugins exist that hand-written registration is repetitive.

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
}
