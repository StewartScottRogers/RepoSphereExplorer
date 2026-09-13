//! Packet capture file type plugin: core and presentation halves.
//!
//! Two formats under one name. The classic one is a twenty-four byte
//! header and then one length-prefixed record per packet, and it carries
//! a link type and a snapshot length and nothing else - no interface
//! names, no comments, nowhere to say which interface a packet arrived
//! on. `pcapng` replaced it with a block structure that can, and the
//! difference is what a reader wants told.
//!
//! The classic header's magic also says the byte order and the timestamp
//! resolution, which is four values in one number.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["pcap", "pcapng", "cap"];

/// The classic magic, written by a machine of the reader's byte order,
/// with timestamps in microseconds.
const CLASSIC: u32 = 0xA1B2_C3D4;

/// The same, with timestamps in nanoseconds.
const CLASSIC_NANOSECOND: u32 = 0xA1B2_3C4D;

/// The block type opening a `pcapng` section.
const SECTION_HEADER: u32 = 0x0A0D_0D0A;

/// The byte-order mark inside a section header block.
const BYTE_ORDER: u32 = 0x1A2B_3C4D;

/// An interface description block.
const INTERFACE_DESCRIPTION: u32 = 0x0000_0001;

/// An enhanced packet block, which is how `pcapng` stores a packet.
const ENHANCED_PACKET: u32 = 0x0000_0006;

/// A simple packet block, which stores one without a timestamp.
const SIMPLE_PACKET: u32 = 0x0000_0003;

/// One interface a capture recorded on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interface {
    /// Its name, when the capture says.
    pub name: Option<String>,
    /// What it is, when the capture says.
    pub description: Option<String>,
    /// The link type, as a number.
    pub link_type: u16,
    /// What that link type is called.
    pub link_reads_as: String,
    /// How much of each packet was kept.
    pub snapshot_length: u32,
}

/// View data produced by [`PcapCore::view`].
///
/// Not `Eq`: the duration is a number of seconds, and a float has no
/// total equality to derive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PcapView {
    /// Which of the two formats this is.
    pub format: String,
    /// Its version.
    pub version: String,
    /// The byte order the file was written in.
    pub byte_order: String,
    /// The interfaces it recorded on. The classic format has exactly
    /// one and cannot name it; `pcapng` may have several and does.
    pub interfaces: Vec<Interface>,
    /// How many packets it holds.
    pub packets: usize,
    /// How many bytes of packet were kept.
    pub captured_bytes: u64,
    /// How many bytes went past on the wire, which is more whenever the
    /// snapshot length cut something short.
    pub wire_bytes: u64,
    /// How many packets were cut short by the snapshot length.
    pub truncated_packets: usize,
    /// The first packet's time, as seconds and microseconds.
    pub first_seen: Option<String>,
    /// The last packet's time.
    pub last_seen: Option<String>,
    /// How long the capture ran, in seconds.
    pub duration_seconds: Option<f64>,
    /// What wrote it, when the capture says. Only `pcapng` can.
    pub written_by: Option<String>,
    /// The machine it was taken on, likewise.
    pub taken_on: Option<String>,
    /// Comments left in the capture, which only `pcapng` can carry.
    pub comments: Vec<String>,
}

/// What a link type is called. Only the ones anybody meets.
fn link_named(link: u16) -> String {
    match link {
        0 => "no link layer".to_owned(),
        1 => "Ethernet".to_owned(),
        3 => "point-to-point".to_owned(),
        6 => "token ring".to_owned(),
        101 => "raw Internet Protocol".to_owned(),
        105 => "802.11 wireless".to_owned(),
        113 => "Linux cooked capture".to_owned(),
        127 => "802.11 with radiotap".to_owned(),
        228 => "raw IPv4".to_owned(),
        229 => "raw IPv6".to_owned(),
        276 => "Linux cooked capture v2".to_owned(),
        other => format!("link type {other}"),
    }
}

/// Whether `prefix` opens like either capture format.
fn looks_like_it(prefix: &[u8]) -> bool {
    let Some(head) = prefix.get(..4) else {
        return false;
    };
    let head: [u8; 4] = head.try_into().unwrap_or_default();
    let forwards = u32::from_le_bytes(head);
    let backwards = u32::from_be_bytes(head);
    forwards == SECTION_HEADER
        || [forwards, backwards].contains(&CLASSIC)
        || [forwards, backwards].contains(&CLASSIC_NANOSECOND)
}

/// A cursor that reads in whichever byte order the file was written in.
struct Reader<'bytes> {
    /// The whole file.
    bytes: &'bytes [u8],
    /// Whether the writer's byte order was the little-endian one.
    little: bool,
}

impl Reader<'_> {
    /// Two bytes at `at`.
    fn u16(&self, at: usize) -> Option<u16> {
        let run: [u8; 2] = self.bytes.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little {
            u16::from_le_bytes(run)
        } else {
            u16::from_be_bytes(run)
        })
    }

    /// Four bytes at `at`.
    fn u32(&self, at: usize) -> Option<u32> {
        let run: [u8; 4] = self.bytes.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little {
            u32::from_le_bytes(run)
        } else {
            u32::from_be_bytes(run)
        })
    }
}

/// A time as seconds and fractions, written out.
fn stamp(seconds: u64, fraction: u64, per_second: u64) -> String {
    format!("{seconds}.{:06}", fraction * 1_000_000 / per_second.max(1))
}

/// Everything [`PcapView`] holds for a classic capture.
fn read_classic(bytes: &[u8], little: bool, nanosecond: bool) -> Option<PcapView> {
    let reader = Reader { bytes, little };
    let per_second = if nanosecond { 1_000_000_000 } else { 1_000_000 };
    let snapshot_length = reader.u32(16)?;
    let link_type = reader.u16(20)?;

    let mut view = PcapView {
        format: "classic pcap".to_owned(),
        version: format!("{}.{}", reader.u16(4)?, reader.u16(6)?),
        byte_order: byte_order_named(little),
        interfaces: vec![Interface {
            // The classic format has no field for either, and saying so
            // is the honest answer rather than inventing a name.
            name: None,
            description: None,
            link_type,
            link_reads_as: link_named(link_type),
            snapshot_length,
        }],
        packets: 0,
        captured_bytes: 0,
        wire_bytes: 0,
        truncated_packets: 0,
        first_seen: None,
        last_seen: None,
        duration_seconds: None,
        written_by: None,
        taken_on: None,
        comments: Vec::new(),
    };

    let mut at = 24usize;
    let mut first = None;
    let mut last = None;
    while at + 16 <= bytes.len() {
        let seconds = u64::from(reader.u32(at)?);
        let fraction = u64::from(reader.u32(at + 4)?);
        let captured = reader.u32(at + 8)? as usize;
        let original = reader.u32(at + 12)?;
        if at + 16 + captured > bytes.len() {
            break;
        }
        view.packets += 1;
        view.captured_bytes += captured as u64;
        view.wire_bytes += u64::from(original);
        if u64::from(original) > captured as u64 {
            view.truncated_packets += 1;
        }
        let moment = (seconds, fraction);
        first.get_or_insert(moment);
        last = Some(moment);
        at += 16 + captured;
    }
    finish(&mut view, first, last, per_second);
    Some(view)
}

/// The byte order, in words.
fn byte_order_named(little: bool) -> String {
    if little {
        "little-endian".to_owned()
    } else {
        "big-endian".to_owned()
    }
}

/// Fills in the times once every packet has been counted.
fn finish(
    view: &mut PcapView,
    first: Option<(u64, u64)>,
    last: Option<(u64, u64)>,
    per_second: u64,
) {
    view.first_seen = first.map(|(seconds, fraction)| stamp(seconds, fraction, per_second));
    view.last_seen = last.map(|(seconds, fraction)| stamp(seconds, fraction, per_second));
    #[expect(
        clippy::cast_precision_loss,
        reason = "a capture whose length needs more than a double's precision would \
                  have to run for longer than there has been packet switching"
    )]
    if let (Some(start), Some(end)) = (first, last) {
        let start = start.0 as f64 + start.1 as f64 / per_second as f64;
        let end = end.0 as f64 + end.1 as f64 / per_second as f64;
        view.duration_seconds = Some(((end - start) * 1_000_000.0).round() / 1_000_000.0);
    }
}

/// The options in a block body, as code and value.
fn options_in(reader: &Reader, from: usize, to: usize) -> Vec<(u16, Vec<u8>)> {
    let mut found = Vec::new();
    let mut at = from;
    while at + 4 <= to {
        let (Some(code), Some(length)) = (reader.u16(at), reader.u16(at + 2)) else {
            break;
        };
        if code == 0 {
            break;
        }
        let length = length as usize;
        let Some(value) = reader.bytes.get(at + 4..at + 4 + length) else {
            break;
        };
        found.push((code, value.to_vec()));
        // Every option is padded out to a four-byte boundary.
        at += 4 + length + (4 - length % 4) % 4;
    }
    found
}

/// One option's value as text.
fn text_option(options: &[(u16, Vec<u8>)], code: u16) -> Option<String> {
    options
        .iter()
        .find(|(had, _)| *had == code)
        .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
}

/// Everything [`PcapView`] holds for a `pcapng` capture.
fn read_pcapng(bytes: &[u8]) -> Option<PcapView> {
    // The byte order is not known until the section header block says,
    // and it says it in a field whose own byte order is the answer.
    let little = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) == BYTE_ORDER;
    let reader = Reader { bytes, little };

    let mut view = PcapView {
        format: "pcapng".to_owned(),
        version: "0.0".to_owned(),
        byte_order: byte_order_named(little),
        interfaces: Vec::new(),
        packets: 0,
        captured_bytes: 0,
        wire_bytes: 0,
        truncated_packets: 0,
        first_seen: None,
        last_seen: None,
        duration_seconds: None,
        written_by: None,
        taken_on: None,
        comments: Vec::new(),
    };
    // Every interface may state its own timestamp resolution, and a
    // packet's timestamp is in its interface's units.
    let mut resolutions: Vec<u64> = Vec::new();
    let mut first = None;
    let mut last = None;
    let mut per_second = 1_000_000u64;

    let mut at = 0usize;
    while at + 12 <= bytes.len() {
        let kind = reader.u32(at)?;
        let length = reader.u32(at + 4)? as usize;
        // A block states its length twice so it can be walked backwards
        // as well; a length that cannot be right stops the walk rather
        // than running off into the rest of the file.
        if length < 12 || at + length > bytes.len() {
            break;
        }
        let body = at + 8;
        let ends = at + length - 4;
        match kind {
            SECTION_HEADER => {
                view.version = format!("{}.{}", reader.u16(body + 4)?, reader.u16(body + 6)?);
                let options = options_in(&reader, body + 16, ends);
                view.taken_on = match (text_option(&options, 2), text_option(&options, 3)) {
                    (Some(hardware), Some(system)) => Some(format!("{hardware}, {system}")),
                    (Some(one), None) | (None, Some(one)) => Some(one),
                    (None, None) => None,
                };
                view.written_by = text_option(&options, 4);
            }
            INTERFACE_DESCRIPTION => {
                let link_type = reader.u16(body)?;
                let options = options_in(&reader, body + 8, ends);
                // `if_tsresol` is a power of ten, or a power of two when
                // its top bit is set.
                let resolution = options
                    .iter()
                    .find(|(code, _)| *code == 9)
                    .and_then(|(_, value)| value.first().copied())
                    .map_or(1_000_000, |power| {
                        if power & 0x80 == 0 {
                            10u64.saturating_pow(u32::from(power))
                        } else {
                            1u64 << (power & 0x7F)
                        }
                    });
                resolutions.push(resolution);
                per_second = resolution;
                view.interfaces.push(Interface {
                    name: text_option(&options, 2),
                    description: text_option(&options, 3),
                    link_type,
                    link_reads_as: link_named(link_type),
                    snapshot_length: reader.u32(body + 4)?,
                });
            }
            ENHANCED_PACKET => {
                let (moment, units) = read_packet(&reader, body, ends, &resolutions, &mut view)?;
                per_second = units;
                first.get_or_insert(moment);
                last = Some(moment);
            }
            SIMPLE_PACKET => {
                let original = reader.u32(body)?;
                view.packets += 1;
                view.captured_bytes += u64::from(original);
                view.wire_bytes += u64::from(original);
            }
            _ => {}
        }
        at += length;
    }
    finish(&mut view, first, last, per_second);
    Some(view)
}

/// Counts one enhanced packet block into `view`, and returns when it
/// was captured together with the units that time is in.
fn read_packet(
    reader: &Reader,
    body: usize,
    ends: usize,
    resolutions: &[u64],
    view: &mut PcapView,
) -> Option<((u64, u64), u64)> {
    let interface = reader.u32(body)? as usize;
    let high = u64::from(reader.u32(body + 4)?);
    let low = u64::from(reader.u32(body + 8)?);
    let captured = reader.u32(body + 12)? as usize;
    let original = reader.u32(body + 16)?;
    view.packets += 1;
    view.captured_bytes += captured as u64;
    view.wire_bytes += u64::from(original);
    if u64::from(original) > captured as u64 {
        view.truncated_packets += 1;
    }
    // A packet's time is in its own interface's units, which is why the
    // interface index is read before the timestamp is divided.
    let units = resolutions.get(interface).copied().unwrap_or(1_000_000);
    let ticks = (high << 32) | low;

    let after = body + 20 + captured + (4 - captured % 4) % 4;
    for (code, value) in options_in(reader, after, ends) {
        if code == 1 {
            view.comments
                .push(String::from_utf8_lossy(&value).into_owned());
        }
    }
    Some(((ticks / units, ticks % units), units))
}

/// Everything [`PcapView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<PcapView> {
    let bytes = std::fs::read(path)?;
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "not a readable packet capture");
    if !looks_like_it(&bytes) {
        return Err(malformed());
    }
    let head: [u8; 4] = bytes
        .get(..4)
        .and_then(|run| run.try_into().ok())
        .ok_or_else(malformed)?;
    let forwards = u32::from_le_bytes(head);
    if forwards == SECTION_HEADER {
        return read_pcapng(&bytes).ok_or_else(malformed);
    }
    let little = forwards == CLASSIC || forwards == CLASSIC_NANOSECOND;
    let nanosecond =
        forwards == CLASSIC_NANOSECOND || u32::from_be_bytes(head) == CLASSIC_NANOSECOND;
    read_classic(&bytes, little, nanosecond).ok_or_else(malformed)
}

/// The packet capture plugin's core half.
#[derive(Debug, Default)]
pub struct PcapCore;

impl PluginCore for PcapCore {
    fn name(&self) -> &'static str {
        "pcap"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, prefix: &[u8]) -> bool {
        looks_like_it(prefix)
    }

    fn view(&self, path: &Path) -> io::Result<Value> {
        let view = read(path)?;
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

/// The packet capture plugin's presentation half.
#[derive(Debug, Default)]
pub struct PcapPresentation;

impl PluginPresentation for PcapPresentation {
    fn name(&self) -> &'static str {
        "pcap"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "PCAP",
            tint: 0x0017_78b5,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: PcapView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![format!(
            "{} {}, {} packet(s), {} byte(s) kept of {} on the wire",
            view.format, view.version, view.packets, view.captured_bytes, view.wire_bytes
        )];
        lines.push(format!("Written {}", view.byte_order));
        if view.truncated_packets == 0 {
            lines.push("No packet was cut short by the snapshot length.".to_owned());
        } else {
            lines.push(format!(
                "{} packet(s) cut short by the snapshot length, so their \
                 tails are not here.",
                view.truncated_packets
            ));
        }
        match (&view.first_seen, &view.last_seen, view.duration_seconds) {
            (Some(first), Some(last), Some(seconds)) => {
                lines.push(format!("From {first} to {last}, {seconds} second(s)"));
            }
            (Some(first), Some(last), None) => lines.push(format!("From {first} to {last}")),
            _ => lines.push("No packet carries a time.".to_owned()),
        }
        if let Some(what) = &view.written_by {
            lines.push(format!("Written by {what}"));
        }
        if let Some(where_taken) = &view.taken_on {
            lines.push(format!("Taken on {where_taken}"));
        }
        lines.push(format!("{} interface(s):", view.interfaces.len()));
        for interface in &view.interfaces {
            lines.push(format!(
                "  {} - {}, snapshot {}",
                interface.name.as_deref().unwrap_or("unnamed"),
                interface.link_reads_as,
                interface.snapshot_length
            ));
            if let Some(description) = &interface.description {
                lines.push(format!("      {description}"));
            }
        }
        if view.interfaces.len() == 1 && view.interfaces[0].name.is_none() {
            lines.push("The classic format has nowhere to record an interface".to_owned());
            lines.push("name, so there is none to show.".to_owned());
        }
        for comment in &view.comments {
            lines.push(format!("Comment: {comment}"));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{PcapCore, PcapPresentation, PcapView, link_named, looks_like_it};
    use plugin_api::{PluginCore, PluginPresentation};

    fn sample(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/pcap")
            .join(name)
    }

    fn view_of(name: &str) -> PcapView {
        serde_json::from_value(PcapCore.view(&sample(name)).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&PcapCore),
            PluginPresentation::extensions(&PcapPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn sniffs_both_formats_and_both_byte_orders() {
        assert!(
            looks_like_it(&[0xd4, 0xc3, 0xb2, 0xa1]),
            "classic, little-endian"
        );
        assert!(
            looks_like_it(&[0xa1, 0xb2, 0xc3, 0xd4]),
            "classic, big-endian"
        );
        assert!(
            looks_like_it(&[0x4d, 0x3c, 0xb2, 0xa1]),
            "classic, nanoseconds"
        );
        assert!(looks_like_it(&[0x0a, 0x0d, 0x0d, 0x0a]), "pcapng");
        assert!(!looks_like_it(b"PK\x03\x04"));
        assert!(!looks_like_it(&[0xd4, 0xc3]));
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn a_link_type_reads_as_what_it_is() {
        assert_eq!(link_named(1), "Ethernet");
        assert_eq!(link_named(113), "Linux cooked capture");
        assert_eq!(link_named(4242), "link type 4242");
    }

    #[test]
    fn reads_the_classic_header_and_every_record() {
        let view = view_of("readings.pcap");

        assert_eq!(view.format, "classic pcap");
        assert_eq!(view.version, "2.4");
        assert_eq!(view.byte_order, "little-endian");
        assert_eq!(view.packets, 4);
        assert_eq!(view.captured_bytes, 258);
        assert_eq!(
            view.wire_bytes, view.captured_bytes,
            "nothing was cut short"
        );
        assert_eq!(view.truncated_packets, 0);
        assert_eq!(view.interfaces.len(), 1);
        assert_eq!(view.interfaces[0].link_reads_as, "Ethernet");
        assert_eq!(view.interfaces[0].snapshot_length, 65535);
    }

    #[test]
    fn the_classic_format_cannot_name_its_interface() {
        let view = view_of("readings.pcap");

        assert_eq!(view.interfaces[0].name, None);
        assert_eq!(view.interfaces[0].description, None);
        assert_eq!(view.written_by, None);
        assert_eq!(view.taken_on, None);
        assert!(view.comments.is_empty());
    }

    #[test]
    fn reads_the_times_and_how_long_the_capture_ran() {
        let view = view_of("readings.pcap");

        assert_eq!(view.first_seen.as_deref(), Some("1789000000.125000"));
        assert_eq!(view.last_seen.as_deref(), Some("1789000002.640100"));
        assert_eq!(view.duration_seconds, Some(2.5151));
    }

    #[test]
    fn reads_what_pcapng_can_say_and_the_classic_format_cannot() {
        let view = view_of("readings.pcapng");

        assert_eq!(view.format, "pcapng");
        assert_eq!(view.version, "1.0");
        assert_eq!(view.written_by.as_deref(), Some("dumpcap 4.2.5"));
        assert_eq!(
            view.taken_on.as_deref(),
            Some("the floor, x86_64, Linux 6.8.0")
        );
        assert_eq!(view.comments.len(), 1);
        assert!(view.comments[0].contains("timed out"));
    }

    #[test]
    fn reads_both_interfaces_with_their_names() {
        let view = view_of("readings.pcapng");

        assert_eq!(view.interfaces.len(), 2);
        assert_eq!(view.interfaces[0].name.as_deref(), Some("eth0"));
        assert_eq!(
            view.interfaces[0].description.as_deref(),
            Some("the wire to the readings host")
        );
        assert_eq!(view.interfaces[0].snapshot_length, 65535);
        assert_eq!(view.interfaces[1].name.as_deref(), Some("lo"));
        assert_eq!(view.interfaces[1].snapshot_length, 262_144);
    }

    #[test]
    fn the_same_packets_count_the_same_in_either_format() {
        let classic = view_of("readings.pcap");
        let ng = view_of("readings.pcapng");

        assert_eq!(classic.packets, ng.packets);
        assert_eq!(classic.captured_bytes, ng.captured_bytes);
        assert_eq!(classic.first_seen, ng.first_seen);
        assert_eq!(classic.last_seen, ng.last_seen);
    }

    #[test]
    fn presents_what_each_format_can_and_cannot_say() {
        let classic = PcapPresentation.present(&PcapCore.view(&sample("readings.pcap")).unwrap());
        let ng = PcapPresentation.present(&PcapCore.view(&sample("readings.pcapng")).unwrap());

        assert!(classic[0].starts_with("classic pcap 2.4, 4 packet(s)"));
        assert!(
            classic
                .iter()
                .any(|line| line.contains("nowhere to record an interface"))
        );
        assert!(!classic.iter().any(|line| line.starts_with("Comment:")));

        assert!(ng[0].starts_with("pcapng 1.0, 4 packet(s)"));
        assert!(ng.iter().any(|line| line.contains("eth0 - Ethernet")));
        assert!(ng.iter().any(|line| line.starts_with("Comment:")));
    }

    #[test]
    fn a_file_that_is_not_a_capture_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.pcap");
        std::fs::write(&path, b"nothing of the sort").unwrap();

        assert!(PcapCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
