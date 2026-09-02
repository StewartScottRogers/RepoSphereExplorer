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

/// The presentation half of a file-type plugin: turns the core half's view
/// data into lines of text a front end can render, without ever touching
/// raw file bytes.
pub trait PluginPresentation: Send + Sync {
    /// The file type's identifier, shared with its core half.
    fn name(&self) -> &'static str;

    /// Turns `data` (as produced by the matching core half) into the lines
    /// a front end should display.
    fn present(&self, data: &serde_json::Value) -> Vec<String>;
}
