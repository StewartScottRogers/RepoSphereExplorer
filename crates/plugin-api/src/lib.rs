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

/// What a run of a file's text is, for colouring it.
///
/// A closed set, and deliberately a small one. This is the lexical layer
/// of the Roslyn C# compiler platform's classification - what a token
/// looks like, decided without knowing what it means - trimmed to the
/// distinctions a reader can tell apart at a glance in a pane a few
/// inches wide. `Type` and `Function` are as far as it reaches; anything
/// finer needs to know what a name refers to, which is a later job
/// (GUIDANCE.md §3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    /// Anything the classifier had nothing to say about, including
    /// whitespace. The gaps between the other classes are these, so that
    /// a run of spans covers its text with no holes in it.
    Plain,
    /// A word the language reserves: `if`, `fn`, `SELECT`.
    Keyword,
    /// A word naming a type: `String`, `int`, `List`.
    Type,
    /// A name being called.
    Function,
    /// A quoted run, including its quotes and anything escaped inside.
    Text,
    /// A numeric literal, including a radix prefix and a suffix.
    Number,
    /// A comment, including the marker that opened it.
    Comment,
    /// Brackets, operators and separators.
    Punctuation,
}

/// One run of a file's text, and what it is.
///
/// `start` and `len` are byte offsets into the text that was classified,
/// and always fall on character boundaries. A classifier returns spans in
/// order, covering the whole text exactly once: no gap, no overlap. That
/// is what lets a front end draw the text by walking the spans and
/// nothing else, and it is checked as a property rather than trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// Byte offset of the run's first character.
    pub start: usize,
    /// The run's length in bytes.
    pub len: usize,
    /// What the run is.
    pub class: Class,
}

impl Span {
    /// A span of `class` covering `start..start + len`.
    #[must_use]
    pub const fn new(start: usize, len: usize, class: Class) -> Self {
        Self { start, len, class }
    }
}

/// One row of the File pane's fact table: a label and its value.
///
/// Structured rather than a sentence, so a front end can lay the value
/// out in its own column, elide it, and show the full text on hover
/// without parsing a sentence back apart to find where the label ends and
/// the value begins (#576).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// What the value is: `"Branch"`, `"Provider"`.
    pub label: String,
    /// The value itself: `"main"`, `"github.com"`.
    pub value: String,
    /// Drawn in the front end's secondary-text colour rather than the
    /// foreground, for a value that should not be read as current or as
    /// important as the rows around it - an old "up to date" that has not
    /// been true since a stale fetch, or a folder count next to a working
    /// copy's own facts.
    pub dim: bool,
}

impl Fact {
    /// A row in the foreground colour: the common case.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            dim: false,
        }
    }
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

    /// The File pane's fact table for `data`: label/value pairs a front
    /// end draws as a two-column table in place of [`Self::present`]'s
    /// lines, whenever this is non-empty.
    ///
    /// Empty by default - most types have nothing tabular to say, and
    /// their [`Self::present`] lines are exactly right on their own. The
    /// directory plugin overrides this for a working copy's provider,
    /// branch, tracking and remote (#576), which used to be sentences
    /// glued together that a narrow File pane wrapped across five lines.
    fn facts(&self, data: &serde_json::Value) -> Vec<Fact> {
        let _ = data;
        Vec::new()
    }

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

    /// What each run of `text` is, for a front end that colours it.
    ///
    /// Empty by default, which reads exactly as it always did: a plugin
    /// that says nothing about its syntax is not a plugin that renders
    /// wrongly. A plugin opts in by describing its language once and
    /// handing the description to the shared tokeniser, so that the
    /// hundred and eighty plugins share one implementation rather than
    /// each growing their own.
    ///
    /// The contract on the returned spans is [`Span`]'s: in order,
    /// covering `text` exactly once. A front end may draw them without
    /// checking, so a classifier that breaks it corrupts the display.
    fn classify(&self, text: &str) -> Vec<Span> {
        let _ = text;
        Vec::new()
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
    use super::{
        Class, Fact, FolderCore, FolderPresentation, Graphic, Icon, PREVIEW_VIEW, PluginCore,
        PluginPresentation, Span, TEXT_VIEW, UNKNOWN_ICON,
    };
    use std::collections::HashSet;
    use std::io;
    use std::path::{Path, PathBuf};

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

    #[test]
    fn a_type_that_does_not_override_facts_has_none() {
        let data = with_content("fn main() {}", false);
        assert_eq!(
            Textish.facts(&data),
            Vec::new(),
            "most types have nothing tabular to say, and present's lines are right on their own"
        );
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

    /// A type that overrides every default this trait offers, so that a
    /// test can tell an override apart from the behaviour it replaced.
    struct Everything;

    impl PluginPresentation for Everything {
        fn name(&self) -> &'static str {
            "everything"
        }
        fn present(&self, _data: &serde_json::Value) -> Vec<String> {
            vec!["preview".to_owned()]
        }
        fn icon(&self) -> Icon {
            Icon {
                label: "EV",
                tint: 0x0012_3456,
            }
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["ev", "evx"]
        }
        fn graphic(&self, _data: &serde_json::Value) -> Option<Graphic> {
            Some(Graphic::Svg("<svg/>".to_owned()))
        }
        fn views(&self, _data: &serde_json::Value) -> Vec<&'static str> {
            vec!["Chart"]
        }
        fn present_view(&self, _view: &str, _data: &serde_json::Value) -> Vec<String> {
            vec!["chart".to_owned()]
        }
        fn classify(&self, _text: &str) -> Vec<Span> {
            vec![Span::new(0, 3, Class::Keyword)]
        }
        fn editable_text(&self, _data: &serde_json::Value) -> Option<String> {
            Some("from the override".to_owned())
        }
        fn facts(&self, _data: &serde_json::Value) -> Vec<Fact> {
            vec![Fact::new("Chart kind", "bar")]
        }
    }

    /// A classifier that breaks the contract [`Span`] documents: its two
    /// spans cover the same bytes twice.
    struct Miscounting;

    impl PluginPresentation for Miscounting {
        fn name(&self) -> &'static str {
            "miscounting"
        }
        fn present(&self, _data: &serde_json::Value) -> Vec<String> {
            Vec::new()
        }
        fn classify(&self, text: &str) -> Vec<Span> {
            vec![
                Span::new(0, text.len(), Class::Keyword),
                Span::new(0, text.len(), Class::Comment),
            ]
        }
    }

    /// A core half that takes both of its defaults.
    struct MinimalCore;

    impl PluginCore for MinimalCore {
        fn name(&self) -> &'static str {
            "minimal"
        }
        fn sniff(&self, prefix: &[u8]) -> bool {
            prefix.starts_with(b"%PDF-")
        }
        fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "content": std::fs::read_to_string(path)? }))
        }
    }

    /// A core half shaped like a lock file: it claims an extension a more
    /// general plugin already owns, and says which plugin it refines.
    struct LockCore;

    impl PluginCore for LockCore {
        fn name(&self) -> &'static str {
            "npm-lock"
        }
        fn sniff(&self, prefix: &[u8]) -> bool {
            prefix.starts_with(b"{\"lockfileVersion\"")
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "packages": 2 }))
        }
        fn extensions(&self) -> &'static [&'static str] {
            &["json"]
        }
        fn specialises(&self) -> &'static [&'static str] {
            &["json"]
        }
    }

    /// The core half of a folder plugin, reading a manifest off the disk.
    struct CargoCore;

    impl FolderCore for CargoCore {
        fn name(&self) -> &'static str {
            "cargo"
        }
        fn sniff(&self, entries: &[&str]) -> bool {
            entries.contains(&"Cargo.toml")
        }
        fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
            let manifest = std::fs::read_to_string(path.join("Cargo.toml"))?;
            Ok(serde_json::json!({ "lines": manifest.lines().count() }))
        }
    }

    /// The presentation half of that same folder plugin.
    struct CargoLines;

    impl FolderPresentation for CargoLines {
        fn name(&self) -> &'static str {
            "cargo"
        }
        fn present(&self, data: &serde_json::Value) -> Vec<String> {
            let lines = data.get("lines").and_then(serde_json::Value::as_u64);
            vec![format!(
                "Cargo project, manifest of {} line(s)",
                lines.unwrap_or_default()
            )]
        }
    }

    /// A second folder plugin, recognising the same folder for a different
    /// reason: this repository's own root is a working copy *and* a Cargo
    /// workspace, and a reader wants both facts.
    struct WorkingCopyCore;

    impl FolderCore for WorkingCopyCore {
        fn name(&self) -> &'static str {
            "working-copy"
        }
        fn sniff(&self, entries: &[&str]) -> bool {
            entries.contains(&".git")
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "branch": "main" }))
        }
    }

    /// The presentation half of the second folder plugin.
    struct WorkingCopyLines;

    impl FolderPresentation for WorkingCopyLines {
        fn name(&self) -> &'static str {
            "working-copy"
        }
        fn present(&self, data: &serde_json::Value) -> Vec<String> {
            let branch = data
                .get("branch")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("(detached)");
            vec![format!("Working copy on {branch}")]
        }
    }

    /// Whether `spans` cover `text` exactly once, in order: the contract
    /// [`Span`] states in prose.
    ///
    /// Written out here rather than borrowed from `syntax::spans_cover`
    /// because `syntax` depends on this crate, so the check cannot be
    /// called from it without a cycle - which is itself why nothing here
    /// enforces the contract on a plugin's `classify`.
    fn covers(text: &str, spans: &[Span]) -> bool {
        let mut at = 0usize;
        for span in spans {
            if span.start != at || !text.is_char_boundary(span.start) {
                return false;
            }
            at = span.start + span.len;
            if at > text.len() || !text.is_char_boundary(at) {
                return false;
            }
        }
        at == text.len()
    }

    /// Whether a front end could draw `graphic`: the guard the graphical
    /// front end applies before handing pixels to Slint, whose buffer
    /// constructor panics on a length that disagrees with its dimensions.
    fn drawable(graphic: &Graphic) -> bool {
        match graphic {
            Graphic::Rgba {
                width,
                height,
                pixels,
            } => {
                let expected = (*width as usize) * (*height as usize) * 4;
                pixels.len() == expected && expected > 0
            }
            Graphic::Svg(source) => !source.is_empty(),
        }
    }

    /// A directory of this test's own under `std::env::temp_dir()`.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("plugin-api-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    /// Every shape of view data a front end could realistically be handed.
    fn every_shape() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({}),
            serde_json::json!({ "content": "text" }),
            serde_json::json!({ "content": "" }),
            serde_json::json!({ "content": null }),
            serde_json::json!({ "content": 7 }),
            serde_json::json!({ "content": ["a"] }),
            serde_json::json!(null),
            serde_json::json!("content"),
            serde_json::json!([1, 2]),
        ]
    }

    #[test]
    fn data_with_no_content_key_at_all_offers_only_the_preview() {
        let data = serde_json::json!({ "pages": 3, "text": "not the key" });
        assert_eq!(
            Binaryish.views(&data),
            vec![PREVIEW_VIEW],
            "the convention is a key named content, not any text anywhere"
        );
    }

    #[test]
    fn content_that_is_not_a_string_offers_only_the_preview() {
        for content in [
            serde_json::json!(7),
            serde_json::json!(true),
            serde_json::json!(["a line"]),
            serde_json::json!({ "text": "a line" }),
        ] {
            let data = serde_json::json!({ "content": content });
            assert_eq!(
                Textish.views(&data),
                vec![PREVIEW_VIEW],
                "content {content} is not the file's text, so there is no text view"
            );
            assert_eq!(
                Textish.present_view(TEXT_VIEW, &data),
                Textish.present(&data),
                "and asking for the text view anyway falls back to the preview"
            );
        }
    }

    #[test]
    fn content_present_but_null_offers_only_the_preview() {
        let data = serde_json::json!({ "content": null, "truncated": false });
        assert_eq!(
            Textish.views(&data),
            vec![PREVIEW_VIEW],
            "a null content is a plugin saying it read no text, not empty text"
        );
        assert_eq!(Textish.editable_text(&data), None);
    }

    #[test]
    fn an_empty_content_string_still_offers_the_text_view() {
        let data = with_content("", false);
        assert_eq!(
            Textish.views(&data),
            vec![PREVIEW_VIEW, TEXT_VIEW],
            "an empty file is a file whose text is empty, and it has one"
        );
        assert!(
            Textish.present_view(TEXT_VIEW, &data).is_empty(),
            "an empty file has no lines to draw"
        );
        assert_eq!(
            Textish.editable_text(&data),
            Some(String::new()),
            "and it is editable, or a file could never be started"
        );
    }

    #[test]
    fn data_that_is_not_an_object_offers_only_the_preview() {
        for data in [
            serde_json::json!(null),
            serde_json::json!("content"),
            serde_json::json!(42),
            serde_json::json!([{ "content": "a line" }]),
        ] {
            assert_eq!(
                Textish.views(&data),
                vec![PREVIEW_VIEW],
                "{data} is not an object, so it carries no content key"
            );
            assert_eq!(Textish.editable_text(&data), None);
        }
    }

    #[test]
    fn the_view_list_is_never_empty_whatever_the_data() {
        for data in every_shape() {
            for (plugin, who) in [
                (&Textish as &dyn PluginPresentation, "textish"),
                (&Binaryish, "binaryish"),
                (&Tabular, "tabular"),
            ] {
                let views = plugin.views(&data);
                assert!(
                    !views.is_empty(),
                    "{who} offered no view at all for {data}; the pane shows \
                     the first one and would have nothing to show"
                );
                assert!(
                    !plugin.present_view(views[0], &data).is_empty(),
                    "{who}'s first view for {data} drew nothing; it is what \
                     the pane shows until the reader asks for another"
                );
            }
        }
    }

    #[test]
    fn the_text_view_is_the_files_lines_without_the_final_empty_one() {
        for (content, lines) in [
            ("one\ntwo\n", vec!["one", "two"]),
            ("one\ntwo", vec!["one", "two"]),
            ("one\n\n", vec!["one", ""]),
            ("\n", vec![""]),
        ] {
            let data = with_content(content, false);
            assert_eq!(
                Textish.present_view(TEXT_VIEW, &data),
                lines,
                "the terminating newline ends the last line, it does not add one"
            );
        }
    }

    #[test]
    fn the_text_view_drops_the_carriage_return_of_a_windows_line_ending() {
        let data = with_content("one\r\ntwo\r\n", false);
        assert_eq!(
            Textish.present_view(TEXT_VIEW, &data),
            vec!["one".to_owned(), "two".to_owned()],
            "a cross-platform explorer draws a checkout made on Windows the \
             same as one made anywhere else"
        );
        assert_eq!(
            Textish.editable_text(&data),
            Some("one\r\ntwo\r\n".to_owned()),
            "but the editor gets the bytes as they are, or saving would \
             rewrite every line ending in the file"
        );
    }

    #[test]
    fn the_view_name_is_matched_exactly_so_a_near_miss_falls_back() {
        let data = with_content("one\ntwo\n", false);
        for asked in ["text", "TEXT", " Text", "Text "] {
            assert_eq!(
                Textish.present_view(asked, &data),
                Textish.present(&data),
                "{asked:?} is not the view name this trait publishes"
            );
        }
        assert_eq!(
            Textish.present_view(TEXT_VIEW, &data),
            vec!["one".to_owned(), "two".to_owned()]
        );
    }

    #[test]
    fn a_view_that_never_mentions_truncation_is_editable() {
        let data = serde_json::json!({ "content": "whole file" });
        assert_eq!(
            Textish.editable_text(&data),
            Some("whole file".to_owned()),
            "absent means not truncated: a view that never truncates has no \
             reason to carry the flag"
        );
    }

    #[test]
    fn a_truncation_flag_that_is_not_a_boolean_leaves_the_view_editable() {
        for flag in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::json!(null),
        ] {
            let data = serde_json::json!({ "content": "first half", "truncated": flag });
            assert_eq!(
                Textish.editable_text(&data),
                Some("first half".to_owned()),
                "only a JSON boolean reads as truncation; {flag} does not"
            );
        }
    }

    #[test]
    fn content_that_is_not_a_string_is_not_editable() {
        let data = serde_json::json!({ "content": { "text": "a line" } });
        assert_eq!(
            Textish.editable_text(&data),
            None,
            "there is no one text to hand an editor"
        );
    }

    #[test]
    fn a_plugin_that_overrides_nothing_takes_every_default() {
        let data = serde_json::json!({ "pages": 3 });
        assert_eq!(
            Binaryish.icon(),
            UNKNOWN_ICON,
            "a plugin that states no mark is marked as unclaimed"
        );
        assert!(
            Binaryish.extensions().is_empty(),
            "and claims no extension, so the sniffer decides alone"
        );
        assert_eq!(Binaryish.graphic(&data), None, "and draws no picture");
        assert!(
            Binaryish.classify("fn main() {}").is_empty(),
            "and says nothing about its syntax, which reads as uncoloured"
        );
        assert_eq!(Binaryish.views(&data), vec![PREVIEW_VIEW]);
        assert_eq!(Binaryish.editable_text(&data), None);
    }

    #[test]
    fn a_plugin_that_overrides_a_default_is_asked_instead_of_it() {
        let data = with_content("fn main() {}", false);
        assert_eq!(
            Everything.icon(),
            Icon {
                label: "EV",
                tint: 0x0012_3456
            }
        );
        assert_eq!(Everything.extensions(), &["ev", "evx"]);
        assert_eq!(
            Everything.graphic(&data),
            Some(Graphic::Svg("<svg/>".to_owned()))
        );
        assert_eq!(
            Everything.views(&data),
            vec!["Chart"],
            "the override decides, even though the data carries content"
        );
        assert_eq!(
            Everything.present_view(TEXT_VIEW, &data),
            vec!["chart".to_owned()],
            "and the content convention no longer answers for the text view"
        );
        assert_eq!(
            Everything.classify("fn main() {}"),
            vec![Span::new(0, 3, Class::Keyword)]
        );
        assert_eq!(
            Everything.editable_text(&data),
            Some("from the override".to_owned()),
            "not the content key the default would have read"
        );
        assert_eq!(
            Everything.facts(&data),
            vec![Fact::new("Chart kind", "bar")],
            "not the empty table the default would have offered"
        );
    }

    #[test]
    fn a_core_that_claims_no_extension_and_refines_nothing_takes_both_defaults() {
        assert!(
            MinimalCore.extensions().is_empty(),
            "a format with magic bytes needs no extension hint"
        );
        assert!(
            MinimalCore.specialises().is_empty(),
            "and refines no other plugin, so nothing overrules the hint"
        );
    }

    #[test]
    fn a_core_that_names_what_it_refines_is_asked_instead_of_the_default() {
        assert_eq!(LockCore.extensions(), &["json"]);
        assert_eq!(
            LockCore.specialises(),
            &["json"],
            "a lock file is JSON, and is the better answer for one"
        );
        assert!(
            LockCore
                .extensions()
                .iter()
                .all(|extension| extension.chars().all(char::is_lowercase)),
            "extensions are stated lowercase and without their dot"
        );
    }

    #[test]
    fn a_core_sniffs_only_the_prefix_it_is_handed() {
        let pdf = b"%PDF-1.7\n1 0 obj\n";
        assert!(MinimalCore.sniff(pdf));
        assert!(
            !MinimalCore.sniff(&pdf[..2]),
            "a prefix too short to hold the magic bytes does not match, so \
             the service has to hand over enough of them"
        );
        assert!(
            !MinimalCore.sniff(b"\xef\xbb\xbf%PDF-1.7"),
            "and the magic bytes are looked for where the format puts them, \
             not anywhere in the prefix"
        );
        assert!(!MinimalCore.sniff(b""), "an empty file matches nothing");
    }

    #[test]
    fn a_core_reading_a_file_that_is_not_there_reports_the_error() {
        let dir = scratch("core-view");
        let missing = dir.join("absent.pdf");
        let error = MinimalCore
            .view(&missing)
            .expect_err("a file that was deleted between listing and opening");
        assert_eq!(
            error.kind(),
            io::ErrorKind::NotFound,
            "the reason travels with the error, so a front end can say which"
        );

        let present = dir.join("present.txt");
        std::fs::write(&present, "one\ntwo\n").expect("a scratch file");
        let data = MinimalCore.view(&present).expect("a file that is there");
        assert_eq!(
            data.get("content").and_then(serde_json::Value::as_str),
            Some("one\ntwo\n")
        );
        std::fs::remove_dir_all(&dir).expect("the scratch directory goes again");
    }

    #[test]
    fn a_span_records_the_run_it_was_built_from() {
        const KEYWORD: Span = Span::new(4, 2, Class::Keyword);
        assert_eq!(KEYWORD.start, 4);
        assert_eq!(KEYWORD.len, 2, "a length, not an end offset");
        assert_eq!(KEYWORD.class, Class::Keyword);
        assert_eq!(
            KEYWORD,
            Span {
                start: 4,
                len: 2,
                class: Class::Keyword
            }
        );
    }

    #[test]
    fn spans_that_meet_exactly_cover_their_text() {
        let text = "let x = 1;";
        assert!(covers("", &[]), "no text needs no spans");
        assert!(
            covers("", &[Span::new(0, 0, Class::Plain)]),
            "and a classifier that emits one empty run for it is still right"
        );
        assert!(covers(text, &[Span::new(0, text.len(), Class::Plain)]));
        assert!(
            covers(
                text,
                &[
                    Span::new(0, 3, Class::Keyword),
                    Span::new(3, 7, Class::Plain),
                ]
            ),
            "the second run starts where the first ended"
        );
        assert!(
            covers(
                text,
                &[
                    Span::new(0, 3, Class::Keyword),
                    Span::new(3, 0, Class::Type),
                    Span::new(3, 7, Class::Plain),
                ]
            ),
            "and a zero-length run between them consumes nothing"
        );
    }

    #[test]
    fn a_gap_an_overlap_or_a_run_out_of_order_breaks_the_cover() {
        let text = "abcd";
        assert!(
            !covers(
                text,
                &[Span::new(0, 1, Class::Plain), Span::new(2, 2, Class::Plain)]
            ),
            "byte 1 is drawn by nothing: the gaps are Plain spans, not holes"
        );
        assert!(
            !covers(
                text,
                &[
                    Span::new(0, 3, Class::Keyword),
                    Span::new(2, 2, Class::Plain)
                ]
            ),
            "byte 2 is drawn twice, once in each colour"
        );
        assert!(
            !covers(
                text,
                &[Span::new(2, 2, Class::Plain), Span::new(0, 2, Class::Plain)]
            ),
            "spans come back in order, and a front end walks them in the \
             order they arrive"
        );
        assert!(
            !covers(text, &[Span::new(0, 2, Class::Plain)]),
            "and stopping short leaves the tail of the file undrawn"
        );
    }

    #[test]
    fn a_span_running_past_the_end_of_the_text_breaks_the_cover() {
        let text = "abcd";
        assert!(!covers(text, &[Span::new(0, text.len() + 1, Class::Plain)]));
        assert!(!covers(
            text,
            &[
                Span::new(0, 4, Class::Plain),
                Span::new(4, 1, Class::Comment),
            ]
        ));
        assert!(
            !covers(text, &[Span::new(0, usize::MAX, Class::Plain)]),
            "a length that would overflow an end offset is out of range too"
        );
    }

    #[test]
    fn a_span_that_splits_a_character_breaks_the_cover() {
        // The failure this prevents is not a wrong colour, it is a panic:
        // slicing a string at a byte inside a multi-byte character.
        let text = "héllo";
        assert_eq!(text.len(), 6, "five characters, six bytes");
        assert!(
            !covers(
                text,
                &[Span::new(0, 2, Class::Plain), Span::new(2, 4, Class::Plain)]
            ),
            "byte 2 is the second half of the e-acute"
        );
        assert!(covers(
            text,
            &[Span::new(0, 3, Class::Plain), Span::new(3, 3, Class::Plain)]
        ));
    }

    #[test]
    fn nothing_here_stops_a_classifier_returning_spans_that_cover_twice() {
        let text = "let x = 1;";
        let spans = Miscounting.classify(text);
        assert_eq!(spans.len(), 2);
        assert!(
            !covers(text, &spans),
            "these spans break the contract Span documents, and this crate \
             hands them to a front end unchecked: the check lives in the \
             syntax crate, which depends on this one, so a plugin that \
             classifies its own format some other way is never asked"
        );
    }

    #[test]
    fn the_classes_are_distinct_so_a_colour_can_be_keyed_by_one() {
        let classes = [
            Class::Plain,
            Class::Keyword,
            Class::Type,
            Class::Function,
            Class::Text,
            Class::Number,
            Class::Comment,
            Class::Punctuation,
        ];
        let distinct: HashSet<Class> = classes.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            classes.len(),
            "a front end keys a colour by class, and two classes that \
             compare equal would draw as one"
        );
    }

    #[test]
    fn pixels_that_disagree_with_the_dimensions_are_not_drawable() {
        let short = Graphic::Rgba {
            width: 4,
            height: 2,
            pixels: vec![0; 4 * 2 * 4 - 1],
        };
        let long = Graphic::Rgba {
            width: 4,
            height: 2,
            pixels: vec![0; 4 * 2 * 4 + 1],
        };
        assert!(!drawable(&short), "one byte short of eight rows of pixels");
        assert!(!drawable(&long), "one byte over");
        assert!(drawable(&Graphic::Rgba {
            width: 4,
            height: 2,
            pixels: vec![0; 4 * 2 * 4],
        }));
    }

    #[test]
    fn a_picture_with_no_width_or_no_height_is_not_drawable() {
        for (width, height) in [(0, 0), (0, 4), (4, 0)] {
            assert!(
                !drawable(&Graphic::Rgba {
                    width,
                    height,
                    pixels: Vec::new(),
                }),
                "{width} by {height} pixels is nothing to draw, and counts \
                 out to an empty buffer that would still be accepted"
            );
        }
        assert!(
            !drawable(&Graphic::Svg(String::new())),
            "and empty SVG source renders as nothing"
        );
    }

    #[test]
    fn nothing_here_stops_a_plugin_miscounting_its_own_pixels() {
        let miscounted = Graphic::Rgba {
            width: 1_000,
            height: 1_000,
            pixels: vec![0; 4],
        };
        assert_eq!(
            miscounted,
            Graphic::Rgba {
                width: 1_000,
                height: 1_000,
                pixels: vec![0; 4],
            },
            "this crate builds it, compares it and carries it without ever \
             checking that the pixels number width * height * 4; the guard \
             that stops it panicking Slint lives in the graphical front end"
        );
        assert!(!drawable(&miscounted));
    }

    #[test]
    fn the_unknown_mark_is_blank_so_a_front_end_draws_a_plain_sheet() {
        assert!(
            UNKNOWN_ICON.label.is_empty(),
            "a file no plugin claims is a document, not an error"
        );
        assert_eq!(
            UNKNOWN_ICON.tint & 0xff00_0000,
            0,
            "a tint is 0xRRGGBB, with nothing in the top byte"
        );
        assert_ne!(
            UNKNOWN_ICON.tint, 0,
            "but it is a colour, or the band would be drawn in black"
        );
    }

    #[test]
    fn nothing_here_stops_an_icon_a_listing_cannot_draw() {
        let overlong = Icon {
            label: "JAVASCRIPT",
            tint: 0xffff_ffff,
        };
        assert!(
            overlong.label.chars().count() > 4,
            "the doc comment says up to four characters and nothing enforces \
             it: a longer label is built, compared and shipped, and the front \
             end sizes its font off a length it never expected"
        );
        assert_ne!(
            overlong.tint & 0xff00_0000,
            0,
            "and a tint outside 0xRRGGBB is accepted just as quietly"
        );
        assert_ne!(overlong, UNKNOWN_ICON);
    }

    #[test]
    fn the_two_view_names_are_distinct_and_neither_is_empty() {
        assert_ne!(
            PREVIEW_VIEW, TEXT_VIEW,
            "present_view tells them apart by name alone"
        );
        assert!(!PREVIEW_VIEW.is_empty());
        assert!(!TEXT_VIEW.is_empty());
        assert!(
            !Tabular
                .views(&with_content("a,b\n", false))
                .contains(&TEXT_VIEW),
            "a plugin that overrides views owns the whole list, so the text \
             view is not quietly added back to it"
        );
    }

    #[test]
    fn every_folder_plugin_that_recognises_a_folder_contributes_its_lines() {
        let entries = ["Cargo.toml", "src", ".git", "README.md"];
        let plugins: [(&dyn FolderCore, &dyn FolderPresentation); 2] = [
            (&CargoCore, &CargoLines),
            (&WorkingCopyCore, &WorkingCopyLines),
        ];

        let already = vec!["4 items".to_owned()];
        let mut shown = already.clone();
        for (core, lines) in plugins {
            assert!(
                core.sniff(&entries),
                "{} recognises this folder, and a folder is several things \
                 at once",
                core.name()
            );
            shown.extend(lines.present(&serde_json::json!({ "lines": 12, "branch": "main" })));
        }

        assert_eq!(
            shown,
            vec![
                "4 items".to_owned(),
                "Cargo project, manifest of 12 line(s)".to_owned(),
                "Working copy on main".to_owned(),
            ],
            "the lines are added below what the folder already reports, \
             never in place of them"
        );
        assert!(shown.starts_with(&already));
    }

    #[test]
    fn a_folder_plugin_sniffs_the_names_directly_inside_the_folder() {
        assert!(CargoCore.sniff(&["Cargo.toml", "src"]));
        assert!(
            !CargoCore.sniff(&["src", "README.md"]),
            "the marker is not there, and sniffing does not walk into src"
        );
        assert!(
            !CargoCore.sniff(&["cargo.toml"]),
            "a manifest is named the way the tool names it"
        );
        assert!(!CargoCore.sniff(&[]), "an empty folder is no project");
        assert!(
            !WorkingCopyCore.sniff(&["Cargo.toml", "src"]),
            "and a plugin that does not recognise a folder contributes nothing"
        );
    }

    #[test]
    fn a_folder_plugin_reading_a_folder_with_no_manifest_reports_the_error() {
        let dir = scratch("folder-view");
        let error = CargoCore
            .view(&dir)
            .expect_err("a folder whose manifest was moved after it was sniffed");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);

        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n")
            .expect("a scratch manifest");
        let data = CargoCore.view(&dir).expect("a manifest that is there");
        assert_eq!(
            CargoLines.present(&data),
            vec!["Cargo project, manifest of 2 line(s)".to_owned()]
        );
        std::fs::remove_dir_all(&dir).expect("the scratch directory goes again");
    }

    #[test]
    fn a_folder_plugin_draws_its_lines_from_data_it_may_not_recognise() {
        assert_eq!(
            WorkingCopyLines.present(&serde_json::json!({})),
            vec!["Working copy on (detached)".to_owned()],
            "a folder pane is the union of several plugins, so one that is \
             handed nothing it knows still has to return lines rather than \
             panic"
        );
        assert_eq!(
            CargoLines.present(&serde_json::json!(null)),
            vec!["Cargo project, manifest of 0 line(s)".to_owned()]
        );
        assert_eq!(
            FolderCore::name(&CargoCore),
            FolderPresentation::name(&CargoLines),
            "the two halves are matched by name, and a mismatch would pair \
             one plugin's data with another's rendering"
        );
    }
}
