//! Minidump file type plugin: core and presentation halves.
//!
//! A minidump is a header, a directory saying where each stream landed,
//! and the streams. Nothing is at a fixed offset except the header, so
//! reading one means reading the directory and following it.
//!
//! What a reader wants from a crash dump is the crash: which thread, what
//! went wrong, where, and what was loaded at the time. Those are four
//! separate streams, and a dump that carries none of them is a dump of
//! nothing - so the pane says which it found.

use plugin_api::{Icon, PluginCore, PluginPresentation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

/// The lowercase extensions this type claims, without their dot.
/// Both halves report these: the presentation half so a listing can
/// mark the file, the core half so `service` can tell this type from
/// another whose content heuristic matches the same text.
pub const EXTENSIONS: &[&str] = &["dmp", "mdmp"];

/// The four bytes every minidump opens with.
const MAGIC: &[u8] = b"MDMP";

/// The header, which the stream directory follows.
const HEADER: usize = 32;

/// One directory entry: the stream's kind, and where it landed.
const DIRECTORY_ENTRY: usize = 12;

/// The stream kinds this reads.
const THREAD_LIST: u32 = 3;
/// The loaded modules and their versions.
const MODULE_LIST: u32 = 4;
/// The exception that ended the process, when one did.
const EXCEPTION: u32 = 6;
/// The machine the process was running on.
const SYSTEM_INFO: u32 = 7;
/// The process identifier and its times.
const MISC_INFO: u32 = 15;

/// One loaded module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    /// Its path on the machine that crashed.
    pub path: String,
    /// Where it was loaded.
    pub base: u64,
    /// How much address space it took.
    pub size: u32,
    /// Its file version, when it carries one.
    pub version: Option<String>,
}

/// The exception that ended the process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exception {
    /// The thread it happened on.
    pub thread_id: u32,
    /// The code, as Windows numbers them.
    pub code: u32,
    /// What that code is called.
    pub reads_as: String,
    /// The instruction it happened at.
    pub address: u64,
    /// The code's own parameters. An access violation puts the kind of
    /// access in the first and the address in the second, which is the
    /// difference between reading a null pointer and writing one.
    pub parameters: Vec<u64>,
}

/// View data produced by [`MinidumpCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinidumpView {
    /// The dump's flags, which say what the writer chose to include.
    pub flags: u64,
    /// What those flags are called.
    pub includes: Vec<String>,
    /// When the dump was taken, as seconds since the epoch.
    pub taken: u32,
    /// The streams the directory names, by name.
    pub streams: Vec<String>,
    /// The process that crashed, when the dump says.
    pub process_id: Option<u32>,
    /// How many threads it had.
    pub thread_count: usize,
    /// The threads' identifiers.
    pub thread_ids: Vec<u32>,
    /// What was loaded.
    pub modules: Vec<Module>,
    /// The exception, when there was one. A dump taken on purpose - by a
    /// debugger, or by the process itself - has none.
    pub exception: Option<Exception>,
    /// The machine, read out.
    pub system: Option<String>,
}

/// How many modules and threads are listed before the rest are counted.
const SHOWN: usize = 64;

/// Whether `prefix` opens like a minidump.
///
/// The signature alone would match any file that happens to start with
/// those four letters, so the directory offset is checked as well: it
/// has to be at least past the header, which is where a real one is.
fn looks_like_it(prefix: &[u8]) -> bool {
    prefix.len() >= HEADER
        && prefix.starts_with(MAGIC)
        && u32_at(prefix, 12).is_some_and(|rva| rva as usize >= HEADER)
}

/// Two bytes at `at`, little-endian.
fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// Four bytes at `at`, little-endian.
fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Eight bytes at `at`, little-endian.
fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// The string at `rva`, which a dump writes as a byte length and then
/// UTF-16.
fn string_at(bytes: &[u8], rva: usize) -> Option<String> {
    let length = u32_at(bytes, rva)? as usize;
    let run = bytes.get(rva + 4..rva + 4 + length)?;
    let (pairs, _) = run.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().copied().map(u16::from_le_bytes).collect();
    Some(String::from_utf16_lossy(&units))
}

/// What a stream kind is called.
fn stream_named(kind: u32) -> String {
    match kind {
        0 => "Unused".to_owned(),
        3 => "ThreadList".to_owned(),
        4 => "ModuleList".to_owned(),
        5 => "MemoryList".to_owned(),
        6 => "Exception".to_owned(),
        7 => "SystemInfo".to_owned(),
        8 => "ThreadExList".to_owned(),
        9 => "Memory64List".to_owned(),
        10 => "CommentA".to_owned(),
        11 => "CommentW".to_owned(),
        12 => "HandleData".to_owned(),
        13 => "FunctionTable".to_owned(),
        14 => "UnloadedModuleList".to_owned(),
        15 => "MiscInfo".to_owned(),
        16 => "MemoryInfoList".to_owned(),
        17 => "ThreadInfoList".to_owned(),
        22 => "SystemMemoryInfo".to_owned(),
        23 => "ProcessVmCounters".to_owned(),
        other => format!("stream {other}"),
    }
}

/// What an exception code is called.
fn exception_named(code: u32) -> String {
    match code {
        0xC000_0005 => "access violation".to_owned(),
        0xC000_001D => "illegal instruction".to_owned(),
        0xC000_0025 => "non-continuable exception".to_owned(),
        0xC000_0026 => "invalid disposition".to_owned(),
        0xC000_008C => "array bounds exceeded".to_owned(),
        0xC000_008D => "floating-point denormal operand".to_owned(),
        0xC000_008E => "floating-point divide by zero".to_owned(),
        0xC000_0094 => "integer divide by zero".to_owned(),
        0xC000_0095 => "integer overflow".to_owned(),
        0xC000_0096 => "privileged instruction".to_owned(),
        0xC000_00FD => "stack overflow".to_owned(),
        0xC000_0374 => "heap corruption".to_owned(),
        0x8000_0003 => "breakpoint".to_owned(),
        0xE063_7274 => "an unhandled C++ exception".to_owned(),
        other => format!("code 0x{other:08x}"),
    }
}

/// What the dump's flags say the writer included.
fn includes_in(flags: u64) -> Vec<String> {
    const NAMED: &[(u64, &str)] = &[
        (0x0001, "data segments"),
        (0x0002, "full memory"),
        (0x0004, "handle data"),
        (0x0008, "filtered memory"),
        (0x0010, "unloaded modules"),
        (0x0020, "indirectly referenced memory"),
        (0x0040, "module paths filtered"),
        (0x0080, "process threads"),
        (0x0100, "private read-write memory"),
        (0x0200, "data filtered"),
        (0x0400, "no optional data"),
        (0x0800, "full memory information"),
        (0x1000, "thread information"),
        (0x2000, "code segments"),
    ];
    NAMED
        .iter()
        .filter(|(bit, _)| flags & bit != 0)
        .map(|(_, name)| (*name).to_owned())
        .collect()
}

/// The architecture a system information stream names.
fn architecture_named(value: u16) -> &'static str {
    match value {
        0 => "x86",
        5 => "ARM",
        6 => "Itanium",
        9 => "x86-64",
        12 => "ARM64",
        _ => "an unrecognised architecture",
    }
}

/// The threads a thread list stream holds.
fn threads_in(bytes: &[u8], rva: usize) -> Vec<u32> {
    let Some(count) = u32_at(bytes, rva) else {
        return Vec::new();
    };
    (0..count as usize)
        .take(SHOWN)
        .filter_map(|index| u32_at(bytes, rva + 4 + index * 48))
        .collect()
}

/// The modules a module list stream holds.
fn modules_in(bytes: &[u8], rva: usize) -> Vec<Module> {
    let Some(count) = u32_at(bytes, rva) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for index in 0..(count as usize).min(SHOWN) {
        let at = rva + 4 + index * 108;
        let (Some(base), Some(size), Some(name_rva)) = (
            u64_at(bytes, at),
            u32_at(bytes, at + 8),
            u32_at(bytes, at + 20),
        ) else {
            break;
        };
        // The version sits in a VS_FIXEDFILEINFO, which is only there
        // if its signature is: a module built without a version
        // resource leaves the whole structure zeroed.
        let version = (u32_at(bytes, at + 24) == Some(0xFEEF_04BD))
            .then(|| {
                let most = u32_at(bytes, at + 32)?;
                let least = u32_at(bytes, at + 36)?;
                Some(format!(
                    "{}.{}.{}.{}",
                    most >> 16,
                    most & 0xFFFF,
                    least >> 16,
                    least & 0xFFFF
                ))
            })
            .flatten();
        found.push(Module {
            path: string_at(bytes, name_rva as usize).unwrap_or_default(),
            base,
            size,
            version,
        });
    }
    found
}

/// The exception an exception stream holds.
fn exception_in(bytes: &[u8], rva: usize) -> Option<Exception> {
    let thread_id = u32_at(bytes, rva)?;
    let code = u32_at(bytes, rva + 8)?;
    let address = u64_at(bytes, rva + 24)?;
    let count = u32_at(bytes, rva + 32)? as usize;
    Some(Exception {
        thread_id,
        code,
        reads_as: exception_named(code),
        address,
        parameters: (0..count.min(15))
            .filter_map(|index| u64_at(bytes, rva + 40 + index * 8))
            .collect(),
    })
}

/// The machine a system information stream describes.
fn system_in(bytes: &[u8], rva: usize) -> Option<String> {
    let architecture = u16_at(bytes, rva)?;
    let processors = *bytes.get(rva + 6)?;
    let major = u32_at(bytes, rva + 8)?;
    let minor = u32_at(bytes, rva + 12)?;
    let build = u32_at(bytes, rva + 16)?;
    Some(format!(
        "Windows {major}.{minor} build {build} on {} with {processors} processor(s)",
        architecture_named(architecture)
    ))
}

/// Everything [`MinidumpView`] holds, read from the file at `path`.
fn read(path: &Path) -> io::Result<MinidumpView> {
    let bytes = std::fs::read(path)?;
    let malformed = || io::Error::new(io::ErrorKind::InvalidData, "not a readable minidump");
    if !looks_like_it(&bytes) {
        return Err(malformed());
    }
    let count = u32_at(&bytes, 8).ok_or_else(malformed)? as usize;
    let directory = u32_at(&bytes, 12).ok_or_else(malformed)? as usize;
    // The header is four letters and two numbers, and a file that opens
    // with those letters by coincidence will have nonsense in the
    // numbers. A directory that does not fit inside the file is the
    // cheapest way to find that out, and the difference between saying
    // "not a minidump" and reporting a dump with nothing in it.
    if count
        .checked_mul(DIRECTORY_ENTRY)
        .and_then(|span| directory.checked_add(span))
        .is_none_or(|end| end > bytes.len())
    {
        return Err(malformed());
    }
    let taken = u32_at(&bytes, 20).ok_or_else(malformed)?;
    let flags = u64_at(&bytes, 24).ok_or_else(malformed)?;

    let mut view = MinidumpView {
        flags,
        includes: includes_in(flags),
        taken,
        streams: Vec::new(),
        process_id: None,
        thread_count: 0,
        thread_ids: Vec::new(),
        modules: Vec::new(),
        exception: None,
        system: None,
    };
    for index in 0..count {
        let at = directory + index * DIRECTORY_ENTRY;
        let (Some(kind), Some(size), Some(rva)) = (
            u32_at(&bytes, at),
            u32_at(&bytes, at + 4),
            u32_at(&bytes, at + 8),
        ) else {
            break;
        };
        let rva = rva as usize;
        // A directory that points past the end is a truncated dump, and
        // the stream is skipped rather than read out of somebody else's
        // bytes.
        if rva + size as usize > bytes.len() {
            continue;
        }
        view.streams.push(stream_named(kind));
        match kind {
            THREAD_LIST => {
                view.thread_count = u32_at(&bytes, rva).unwrap_or(0) as usize;
                view.thread_ids = threads_in(&bytes, rva);
            }
            MODULE_LIST => view.modules = modules_in(&bytes, rva),
            EXCEPTION => view.exception = exception_in(&bytes, rva),
            SYSTEM_INFO => view.system = system_in(&bytes, rva),
            // The process identifier is only there when the stream's
            // flag word says it is.
            MISC_INFO if u32_at(&bytes, rva + 4).is_some_and(|valid| valid & 0b1 != 0) => {
                view.process_id = u32_at(&bytes, rva + 8);
            }
            _ => {}
        }
    }
    Ok(view)
}

/// The minidump plugin's core half.
#[derive(Debug, Default)]
pub struct MinidumpCore;

impl PluginCore for MinidumpCore {
    fn name(&self) -> &'static str {
        "minidump"
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

/// The minidump plugin's presentation half.
#[derive(Debug, Default)]
pub struct MinidumpPresentation;

impl PluginPresentation for MinidumpPresentation {
    fn name(&self) -> &'static str {
        "minidump"
    }

    fn icon(&self) -> Icon {
        Icon {
            label: "DMP",
            tint: 0x00b0_3a2e,
        }
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn present(&self, data: &Value) -> Vec<String> {
        let view: MinidumpView = match serde_json::from_value(data.clone()) {
            Ok(view) => view,
            Err(err) => return vec![format!("could not read view data: {err}")],
        };
        let mut lines = vec![match view.process_id {
            Some(pid) => format!("Minidump of process {pid}, taken at {}", view.taken),
            None => format!("Minidump taken at {}", view.taken),
        }];
        if let Some(system) = &view.system {
            lines.push(system.clone());
        }
        match &view.exception {
            Some(exception) => {
                lines.push(format!(
                    "Crashed: {} on thread 0x{:04x} at 0x{:x}",
                    exception.reads_as, exception.thread_id, exception.address
                ));
                // An access violation says what it was doing and where,
                // and that is usually the whole diagnosis.
                if exception.code == 0xC000_0005 && exception.parameters.len() >= 2 {
                    let doing = match exception.parameters[0] {
                        0 => "reading",
                        1 => "writing",
                        8 => "executing",
                        _ => "accessing",
                    };
                    lines.push(format!("  {doing} 0x{:x}", exception.parameters[1]));
                }
            }
            None => lines.push("No exception: this dump was taken on purpose.".to_owned()),
        }
        lines.push(format!(
            "{} thread(s): {}",
            view.thread_count,
            view.thread_ids
                .iter()
                .map(|id| format!("0x{id:04x}"))
                .collect::<Vec<String>>()
                .join(", ")
        ));
        if view.includes.is_empty() {
            lines.push("The flags claim nothing beyond the streams themselves.".to_owned());
        } else {
            lines.push(format!("Includes {}", view.includes.join(", ")));
        }
        lines.push(format!("Streams: {}", view.streams.join(", ")));
        if !view.modules.is_empty() {
            lines.push(format!("{} module(s) loaded:", view.modules.len()));
            for module in &view.modules {
                let version = module.version.as_deref().unwrap_or("no version");
                lines.push(format!(
                    "  0x{:012x} {:>9} {}  {version}",
                    module.base, module.size, module.path
                ));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MinidumpCore, MinidumpPresentation, MinidumpView, exception_named, includes_in,
        looks_like_it,
    };
    use plugin_api::{PluginCore, PluginPresentation};

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../samples/minidump/csvstats.dmp")
    }

    fn view_of() -> MinidumpView {
        serde_json::from_value(MinidumpCore.view(&fixture()).unwrap()).unwrap()
    }

    #[test]
    fn both_halves_claim_the_same_extensions() {
        assert_eq!(
            PluginCore::extensions(&MinidumpCore),
            PluginPresentation::extensions(&MinidumpPresentation),
            "one list, or a listing marks a file with a type its viewer will not open"
        );
    }

    #[test]
    fn the_signature_alone_is_not_enough() {
        let mut head = [0u8; 32];
        head[..4].copy_from_slice(b"MDMP");
        head[12] = 32;
        assert!(looks_like_it(&head));

        head[12] = 4;
        assert!(
            !looks_like_it(&head),
            "a directory inside the header is not a directory"
        );
        assert!(!looks_like_it(b"MDMP"), "four bytes is not a header");
        assert!(!looks_like_it(b""));
    }

    #[test]
    fn an_exception_code_reads_as_what_it_is() {
        assert_eq!(exception_named(0xC000_0005), "access violation");
        assert_eq!(exception_named(0xC000_00FD), "stack overflow");
        assert_eq!(exception_named(0x1234), "code 0x00001234");
    }

    #[test]
    fn the_flag_word_says_what_the_writer_included() {
        assert_eq!(includes_in(0x0002), vec!["full memory"]);
        assert_eq!(includes_in(0x0003), vec!["data segments", "full memory"]);
        assert!(includes_in(0).is_empty());
    }

    #[test]
    fn reads_the_header_and_the_directory() {
        let view = view_of();

        assert_eq!(view.taken, 1_789_000_000);
        assert_eq!(view.includes, vec!["full memory"]);
        assert_eq!(
            view.streams,
            vec![
                "ThreadList",
                "ModuleList",
                "Exception",
                "SystemInfo",
                "MiscInfo"
            ]
        );
        assert_eq!(view.process_id, Some(7300));
    }

    #[test]
    fn reads_the_threads() {
        let view = view_of();

        assert_eq!(view.thread_count, 4);
        assert_eq!(view.thread_ids, vec![0x1A2C, 0x1A30, 0x1A34, 0x1A38]);
    }

    #[test]
    fn reads_the_modules_with_their_versions() {
        let view = view_of();

        assert_eq!(view.modules.len(), 4);
        let first = &view.modules[0];
        assert!(first.path.ends_with("csvstats.exe"), "{}", first.path);
        assert_eq!(first.base, 0x0000_7FF6_1234_0000);
        assert_eq!(first.size, 0x0002_4000);
        assert_eq!(first.version.as_deref(), Some("1.0.3.0"));
        assert!(
            view.modules
                .iter()
                .any(|module| module.path.ends_with("ntdll.dll")
                    && module.version.as_deref() == Some("10.0.22621.3155"))
        );
    }

    #[test]
    fn reads_the_exception_and_its_parameters() {
        let view = view_of();

        let exception = view.exception.expect("the dump carries one");
        assert_eq!(exception.code, 0xC000_0005);
        assert_eq!(exception.reads_as, "access violation");
        assert_eq!(exception.thread_id, 0x1A2C);
        assert_eq!(exception.address, 0x0000_7FF6_1234_5678);
        assert_eq!(
            exception.parameters,
            vec![0, 0x10],
            "an access violation says what it was doing, then where"
        );
    }

    #[test]
    fn reads_the_machine_it_ran_on() {
        let view = view_of();

        let system = view.system.expect("the dump carries system information");
        assert!(system.contains("x86-64"), "{system}");
        assert!(system.contains("build 22621"), "{system}");
        assert!(system.contains("16 processor(s)"), "{system}");
    }

    #[test]
    fn presents_the_crash_as_the_first_thing_a_reader_needs() {
        let data = MinidumpCore.view(&fixture()).unwrap();

        let lines = MinidumpPresentation.present(&data);

        assert!(lines[0].starts_with("Minidump of process 7300"));
        assert!(
            lines
                .iter()
                .any(|line| line.contains("Crashed: access violation"))
        );
        assert!(
            lines.iter().any(|line| line.contains("reading 0x10")),
            "reading a null pointer plus sixteen is the diagnosis, and it \
             has to be said in words"
        );
        assert!(lines.iter().any(|line| line.contains("ntdll.dll")));
    }

    #[test]
    fn a_directory_that_does_not_fit_in_the_file_is_not_a_dump() {
        // Four letters and a plausible-looking offset is all `sniff` can
        // see. Without the check in `read`, this reported a dump with no
        // streams in it rather than saying it was not one.
        let mut head = vec![0u8; 32];
        head[..4].copy_from_slice(b"MDMP");
        head[8..12].copy_from_slice(&9999u32.to_le_bytes());
        head[12..16].copy_from_slice(&32u32.to_le_bytes());
        let path = std::env::temp_dir().join("truncated.dmp");
        std::fs::write(&path, &head).unwrap();

        assert!(looks_like_it(&head), "it gets past the sniff");
        assert!(
            MinidumpCore.view(&path).is_err(),
            "and is caught by the read"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_a_dump_is_an_error_rather_than_a_panic() {
        let path = std::env::temp_dir().join("not-really-a.dmp");
        std::fs::write(&path, b"MDMP and then nothing of the sort at all").unwrap();

        assert!(MinidumpCore.view(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
