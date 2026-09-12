//! tar archive file type plugin: core and presentation halves.
//!
//! A tar archive is a sequence of five-hundred-and-twelve byte headers,
//! each followed by its file's bytes padded to the same block size. This
//! reads every entry with its path, size, mode, owner and kind, the
//! total, which variant of the format was used, and the extended headers
//! a path or a size too big for the original fields needs.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, Read};
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
pub const EXTENSIONS: &[&str] = &["tar"];

/// Every header and every run of file bytes is padded to this.
const BLOCK: usize = 512;

/// Where the format marker sits in a header.
const MAGIC_AT: usize = 257;

/// How many entries are listed before the rest are only counted.
const SHOWN: usize = 64;

/// One entry in the archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Its path within the archive.
    pub path: String,
    /// What it is: a file, a directory, a symbolic link and so on.
    pub kind: String,
    /// Its size in bytes. A directory and a link have none.
    pub size: u64,
    /// Its permissions, as they would be written for `chmod`.
    pub mode: String,
    /// Who owns it, as the archive records them.
    pub owner: String,
    /// What a link points at.
    pub link_target: Option<String>,
}

/// View data produced by [`TarCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TarView {
    /// Which variant of the format wrote it.
    pub format: String,
    /// How many entries there are.
    pub entries: usize,
    /// The first of them.
    pub listed: Vec<Entry>,
    /// The uncompressed total of every file in it.
    pub total_size: u64,
    /// The extended headers used, and what each was needed for.
    pub extended_headers: Vec<String>,
    /// Entries whose path climbs out of the directory it is unpacked
    /// into, which overwrite whatever they land on.
    pub escaping_paths: Vec<String>,
    /// Entries writable by anyone, or with the set-user-identity bit,
    /// which are the modes worth noticing before unpacking.
    pub risky_modes: Vec<String>,
}

/// The kinds an entry may be, by the byte in its header.
fn kind_of(marker: u8) -> &'static str {
    match marker {
        b'0' | b'\0' => "file",
        b'1' => "hard link",
        b'2' => "symbolic link",
        b'3' => "character device",
        b'4' => "block device",
        b'5' => "directory",
        b'6' => "named pipe",
        b'7' => "contiguous file",
        b'x' | b'X' => "extended header for the entry after it",
        b'g' => "extended header for the whole archive",
        b'L' => "a long name for the entry after it",
        b'K' => "a long link target for the entry after it",
        _ => "unrecognised",
    }
}

/// A null-padded string field.
fn text(block: &[u8], at: usize, length: usize) -> String {
    let slice = block.get(at..at + length).unwrap_or_default();
    let end = slice
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(slice.len());
    String::from_utf8_lossy(&slice[..end]).trim().to_owned()
}

/// A numeric field, which tar writes in octal.
fn octal(block: &[u8], at: usize, length: usize) -> u64 {
    u64::from_str_radix(&text(block, at, length), 8).unwrap_or(0)
}

/// Whether `block`'s own checksum field matches the bytes around it.
///
/// This is the only test that tells a tar header from five hundred and
/// twelve bytes of anything: the checksum is the sum of every byte with
/// its own field read as spaces.
fn checksum_matches(block: &[u8]) -> bool {
    if block.len() < BLOCK {
        return false;
    }
    let declared = octal(block, 148, 8);
    let mut sum = 0u64;
    for (at, byte) in block.iter().enumerate().take(BLOCK) {
        sum += if (148..156).contains(&at) {
            u64::from(b' ')
        } else {
            u64::from(*byte)
        };
    }
    declared != 0 && sum == declared
}

/// The permissions, written the way `chmod` takes them.
fn mode_of(mode: u64) -> String {
    format!("{:04o}", mode & 0o7777)
}

/// Whether `path` climbs out of wherever it is unpacked.
fn escapes(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || path.split(['/', '\\']).any(|part| part == "..")
        // A Windows drive letter is an absolute path too.
        || path.chars().nth(1) == Some(':')
}

/// Whether `mode` is one worth noticing before unpacking.
fn risky(mode: u64, kind: &str) -> Option<&'static str> {
    if mode & 0o4000 != 0 {
        return Some("runs as its owner, whoever unpacks it");
    }
    if mode & 0o2000 != 0 {
        return Some("runs as its group, whoever unpacks it");
    }
    // A symbolic link is 0777 by convention and means nothing by it.
    if kind != "symbolic link" && mode & 0o002 != 0 {
        return Some("writable by anyone on the machine");
    }
    None
}

/// Everything [`TarView`] holds, read from `bytes`.
fn parse(bytes: &[u8]) -> Option<TarView> {
    let mut view = TarView {
        format: "the original, from before either extension".to_owned(),
        entries: 0,
        listed: Vec::new(),
        total_size: 0,
        extended_headers: Vec::new(),
        escaping_paths: Vec::new(),
        risky_modes: Vec::new(),
    };
    let mut at = 0usize;
    let mut pending_long_name: Option<String> = None;

    while let Some(block) = bytes.get(at..at + BLOCK) {
        // Two blocks of zeroes end the archive.
        if block.iter().all(|byte| *byte == 0) {
            break;
        }
        if !checksum_matches(block) {
            // A header that does not add up is where a truncated archive
            // stops, not a reason to throw the rest away.
            break;
        }
        let magic = text(block, MAGIC_AT, 6);
        if magic == "ustar" {
            view.format = if text(block, 263, 2) == "00" {
                "POSIX, or PAX where an extended header appears".to_owned()
            } else {
                "GNU".to_owned()
            };
        }

        let marker = *block.get(156).unwrap_or(&b'0');
        let kind = kind_of(marker);
        let size = octal(block, 124, 12);
        let prefix = text(block, 345, 155);
        let name = text(block, 0, 100);
        let path = match pending_long_name.take() {
            Some(long) => long,
            None if prefix.is_empty() => name,
            None => format!("{prefix}/{name}"),
        };

        at += BLOCK;
        let stored = usize::try_from(size).ok()?;
        let padded = stored.div_ceil(BLOCK) * BLOCK;

        match marker {
            b'x' | b'X' | b'g' => {
                // The extended header's own body says what it carries -
                // including, when the path is over a hundred bytes, the
                // whole path. The header after it holds only as much of
                // the name as fitted, so the body is the authority.
                let body = String::from_utf8_lossy(bytes.get(at..at + stored).unwrap_or_default());
                let (reasons, extended_path) = records_in(&body);
                for reason in reasons {
                    if !view.extended_headers.contains(&reason) {
                        view.extended_headers.push(reason);
                    }
                }
                if let Some(said) = extended_path {
                    pending_long_name = Some(said);
                }
                at += padded;
                continue;
            }
            b'L' => {
                let body = bytes.get(at..at + stored).unwrap_or_default();
                pending_long_name = Some(
                    String::from_utf8_lossy(body)
                        .trim_end_matches('\0')
                        .to_owned(),
                );
                if !view
                    .extended_headers
                    .iter()
                    .any(|said| said.contains("GNU long name"))
                {
                    view.extended_headers
                        .push("a GNU long name, for a path over a hundred bytes".to_owned());
                }
                at += padded;
                continue;
            }
            _ => {}
        }

        view.entries += 1;
        view.total_size += size;
        let mode = octal(block, 100, 8);
        if escapes(&path) {
            view.escaping_paths.push(path.clone());
        }
        if let Some(why) = risky(mode, kind) {
            view.risky_modes
                .push(format!("{path} ({}): {why}", mode_of(mode)));
        }
        if view.listed.len() < SHOWN {
            view.listed.push(Entry {
                path,
                kind: kind.to_owned(),
                size,
                mode: mode_of(mode),
                owner: format!("{}:{}", text(block, 265, 32), text(block, 297, 32)),
                link_target: (marker == b'1' || marker == b'2')
                    .then(|| text(block, 157, 100))
                    .filter(|target| !target.is_empty()),
            });
        }
        at += padded;
    }
    (view.entries > 0).then_some(view)
}

/// What an extended header carries: why it was needed, and the path it
/// holds when it holds one.
///
/// Each record is `<length> <keyword>=<value>` and a newline, where the
/// length counts itself - so a value may hold spaces, and splitting on
/// whitespace loses any path with one in it.
fn records_in(body: &str) -> (Vec<String>, Option<String>) {
    let mut reasons = Vec::new();
    let mut path = None;
    let mut rest = body;

    while let Some(space) = rest.find(' ') {
        let Ok(length) = rest[..space].parse::<usize>() else {
            break;
        };
        let Some(record) = rest.get(space + 1..length) else {
            break;
        };
        rest = &rest[length.min(rest.len())..];

        let (key, value) = record.split_once('=').unwrap_or((record, ""));
        let said = match key {
            "path" => {
                path = Some(value.trim_end_matches('\n').to_owned());
                "an extended path, for a name over a hundred bytes"
            }
            "linkpath" => "an extended link target, for one over a hundred bytes",
            "size" => "an extended size, for a file over eight gigabytes",
            "uid" | "gid" => "an extended owner, for an identifier over seven digits",
            "mtime" => "an extended modification time, for one below a whole second",
            "uname" | "gname" => "an extended owner name, for one over thirty-two bytes",
            _ => continue,
        };
        reasons.push(said.to_owned());
    }
    (reasons, path)
}

/// Whether `prefix` opens like a tar archive.
fn looks_like_it(prefix: &[u8]) -> bool {
    // The marker alone is not enough: it is five bytes of ASCII a quarter
    // of the way into any file. The header has to add up as well.
    prefix.len() >= BLOCK
        && checksum_matches(prefix)
        && (text(prefix, MAGIC_AT, 6) == "ustar" || parse(prefix).is_some())
}

/// The tar archive plugin's core half.
#[derive(Debug, Default)]
pub struct TarCore;

impl PluginCore for TarCore {
    fn name(&self) -> &'static str {
        "tar"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        // An archive is read whole: the headers are spread through it,
        // not gathered at either end.
        let mut handle = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        handle.read_to_end(&mut bytes)?;
        let view = parse(&bytes)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no readable tar header"))?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The tar archive plugin's presentation half.
#[derive(Debug, Default)]
pub struct TarPresentation;

impl PluginPresentation for TarPresentation {
    fn name(&self) -> &'static str {
        "tar"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "TAR",
            tint: 0x008b_6f47,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: TarView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![
            format!(
                "tar: {} entry(ies), {} byte(s) unpacked",
                view.entries, view.total_size
            ),
            format!("Format: {}", view.format),
        ];
        for entry in &view.listed {
            let target = entry
                .link_target
                .as_ref()
                .map_or_else(String::new, |said| format!(" -> {said}"));
            lines.push(format!(
                "  {} {:>10} {} {}{target}",
                entry.mode, entry.size, entry.owner, entry.path
            ));
        }
        if view.entries > view.listed.len() {
            lines.push(format!(
                "  ... and {} more",
                view.entries - view.listed.len()
            ));
        }
        if !view.extended_headers.is_empty() {
            lines.push("Extended headers, and what each was needed for:".to_owned());
            for said in &view.extended_headers {
                lines.push(format!("  {said}"));
            }
        }
        if !view.escaping_paths.is_empty() {
            lines.push("Climbs out of wherever this is unpacked, so it would".to_owned());
            lines.push("overwrite whatever it lands on:".to_owned());
            for path in &view.escaping_paths {
                lines.push(format!("  {path}"));
            }
        }
        if !view.risky_modes.is_empty() {
            lines.push("Worth reading before unpacking:".to_owned());
            for said in &view.risky_modes {
                lines.push(format!("  {said}"));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{TarCore, TarPresentation, TarView, escapes, looks_like_it, parse, risky};
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/tar/readings.tar")
    }

    fn view_of() -> TarView {
        serde_json::from_value(TarCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&TarCore),
            PluginPresentation::extensions(&TarPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_a_real_archive() {
        let bytes = std::fs::read(fixture()).unwrap();

        assert!(looks_like_it(&bytes));
    }

    #[test]
    fn the_marker_alone_is_not_enough() {
        // `ustar` at the right offset, and nothing else right.
        let mut bytes = vec![0u8; 512];
        bytes[257..262].copy_from_slice(b"ustar");

        assert!(
            !looks_like_it(&bytes),
            "the checksum has to add up as well, or any file with those five \
             bytes a quarter of the way in is an archive"
        );
    }

    #[test]
    fn does_not_claim_a_compressed_archive() {
        let compressed = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/gzip/readings.tar.gz");
        let bytes = std::fs::read(compressed).unwrap();

        assert!(
            !looks_like_it(&bytes),
            "tar's header sits at offset 257 of the plain bytes, and there are none"
        );
    }

    #[test]
    fn reads_every_entry_with_its_mode_and_owner() {
        let view = view_of();

        assert!(view.entries >= 6);
        assert!(view.listed.iter().any(|e| e.kind == "directory"));
        assert!(view.listed.iter().any(|e| e.kind == "symbolic link"));
        let secret = view
            .listed
            .iter()
            .find(|e| e.path.ends_with(".secret"))
            .unwrap();
        assert_eq!(secret.mode, "0600");
        assert_eq!(secret.owner, "floor:floor");
    }

    #[test]
    fn a_symbolic_link_keeps_what_it_points_at() {
        let view = view_of();

        let link = view
            .listed
            .iter()
            .find(|e| e.kind == "symbolic link")
            .unwrap();
        assert_eq!(link.link_target.as_deref(), Some("raw/samples.csv"));
    }

    #[test]
    fn an_extended_header_is_not_an_entry() {
        let view = view_of();

        assert!(
            !view
                .listed
                .iter()
                .any(|e| e.kind.contains("extended header")),
            "a PAX header describes the entry after it; it is not one itself"
        );
        assert!(
            view.extended_headers
                .iter()
                .any(|said| said.contains("extended path")),
            "the buried file's path is over a hundred bytes: {:?}",
            view.extended_headers
        );
        assert!(view.listed.iter().any(|e| e.path.contains("buried.txt")));
    }

    #[test]
    fn an_extended_record_counts_its_own_length() {
        use super::records_in;

        // The length counts itself, the space, the record and the
        // newline - which makes it a fixed point, and easy to write
        // wrongly by hand.
        let body = "32 path=readings/a b/buried.txt\n22 mtime=1789000000.0\n";
        let (reasons, path) = records_in(body);

        assert_eq!(
            path.as_deref(),
            Some("readings/a b/buried.txt"),
            "a path may hold a space, so splitting on whitespace loses it"
        );
        assert_eq!(reasons.len(), 2);
    }

    #[test]
    fn names_a_path_that_climbs_out() {
        assert!(escapes("/etc/passwd"));
        assert!(escapes("../../etc/passwd"));
        assert!(escapes("C:/Windows/System32"));
        assert!(!escapes("readings/raw/samples.csv"));
        assert!(
            !escapes("readings/a..b/file"),
            "two dots inside a name are not a climb"
        );
    }

    #[test]
    fn a_symbolic_links_mode_is_not_reported() {
        assert!(risky(0o777, "symbolic link").is_none(), "they are all 0777");
        assert!(risky(0o777, "file").is_some());
        assert!(risky(0o4755, "file").is_some_and(|said| said.contains("as its owner")));
        assert!(risky(0o644, "file").is_none());
    }

    #[test]
    fn presents_the_extended_header_with_its_reason() {
        let data = TarCore.view(&fixture()).unwrap();

        let lines = TarPresentation.present(&data);

        assert!(lines[0].starts_with("tar: "));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("over a hundred bytes"))
        );
        assert!(lines.iter().any(|line| line.contains("0600")));
    }

    #[test]
    fn a_truncated_archive_gives_up_what_it_read() {
        let mut bytes = std::fs::read(fixture()).unwrap();
        bytes.truncate(1536);

        let view = parse(&bytes).unwrap();
        assert!(
            view.entries >= 1,
            "the headers before the cut are still headers"
        );
    }

    #[test]
    fn a_file_that_is_not_tar_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really.tar");
        std::fs::write(&path, b"nothing like an archive").unwrap();

        assert!(TarCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
