//! IPC message types, versioned, shared by the service and both front ends.

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Name, NameType, ToFsName, ToNsName,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// The protocol version this build speaks. Bump whenever [`Request`] or
/// [`Response`] changes shape in a way that is not backward compatible.
pub const VERSION: u32 = 2;

/// The name other processes use to find the service's local socket.
pub const SOCKET_NAME: &str = "reposphereexplorer.sock";

/// The largest message [`read_message`] will accept, in bytes. The length
/// prefix arrives before any of the payload it describes, so without a cap
/// four bytes are enough to make the reader commit up to 4 GiB. Real
/// messages are far smaller - a file view is capped at 64 KiB by its
/// plugin, and the largest thing on the wire is a directory listing.
pub const MAX_MESSAGE_BYTES: u32 = 64 * 1024 * 1024;

/// The deepest nesting a message may carry.
///
/// `serde_json` refuses to *read* past 128 levels and had no matching rule
/// for writing, so this crate could write a message its own peer refused:
/// measured, 125 levels round-tripped and 126 was written and then rejected
/// as invalid data. Reachable through the JSON plugin, which puts a whole
/// parsed document into the view it sends.
///
/// Held on the writing side, where a refusal can still be turned into an
/// answer that names the reason, instead of surfacing as a generic failure
/// across a process boundary.
pub const MAX_MESSAGE_DEPTH: usize = 127;

/// A request sent from a front end to the service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    /// List the immediate contents of a directory.
    ListDirectory {
        /// Path to list, as given by the caller.
        path: String,
    },
    /// View a single file's content through its recognised plugin.
    ViewFile {
        /// Path to view, as given by the caller.
        path: String,
    },
    /// Open `path`: lists it if it is a directory, otherwise views it
    /// through its recognised plugin.
    Open {
        /// Path to open, as given by the caller.
        path: String,
    },
    /// Renames (or moves) every pair in `items`, each an existing path and
    /// the path it should have afterwards. Journaled, and undone as one
    /// operation: D6 settles batch operations and one-step undo together,
    /// so a move of several files is one thing a reader did and one thing
    /// Ctrl+Z puts back.
    Rename {
        /// The pairs to move, source first.
        items: Vec<(String, String)>,
    },
    /// Copies every pair in `items`, each a source file and its
    /// destination. Journaled and undone as one operation, as [`Self::Rename`]
    /// is.
    Copy {
        /// The pairs to copy, source first.
        items: Vec<(String, String)>,
    },
    /// Deletes every path in `paths`: the exact, confirmed target set (per
    /// GUIDANCE.md §2.1.5, not a pattern the service resolves itself).
    /// Journaled.
    Delete {
        /// The exact paths to delete.
        paths: Vec<String>,
    },
    /// Extracts the archive at `archive` into `destination`. Journaled.
    Extract {
        /// The archive to extract.
        archive: String,
        /// The directory to extract into.
        destination: String,
    },
    /// Creates a new, empty directory at `path`. Journaled.
    CreateDirectory {
        /// The path of the directory to create.
        path: String,
    },
    /// The configured Repos Directory roots, as stored on this machine.
    ///
    /// The reply is [`Response::ReposRoots`], whose list is empty when
    /// nothing has been configured yet - which is what a first run looks
    /// like.
    ReposRoots,
    /// Makes `path` the active Repos Directory, adding it to the stored
    /// list if it is not already there. Journaled.
    SetReposRoot {
        /// The directory to open at from now on.
        path: String,
    },
    /// Replaces the text of an existing file. Journaled, and undoable: the
    /// service keeps what was there before.
    WriteFile {
        /// The file to write. Must already exist; this never creates one.
        path: String,
        /// Its new contents.
        content: String,
    },
    /// Undoes the immediately preceding operation, if it can be undone.
    /// The service holds what that is; a front end only asks.
    Undo,
    /// Creates a new, empty file at `path`. Journaled.
    CreateFile {
        /// The path of the file to create.
        path: String,
    },
}

/// One entry returned by [`Request::ListDirectory`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    /// File or directory name, without its parent path.
    pub name: String,
    /// Whether the entry is itself a directory.
    pub is_dir: bool,
    /// Size in bytes, from the entry's own metadata. For a directory this
    /// is the directory metadata's size, not a recursive sum of contents.
    pub size: u64,
    /// Last modified time, in seconds since `UNIX_EPOCH`. `None` if the
    /// platform or filesystem doesn't report one.
    pub modified: Option<u64>,
    /// What this entry is, as a source control working directory. `None`
    /// for a file, and for a directory that is not a working copy - which
    /// stays listed either way, per GUIDANCE.md §2.5.
    #[serde(default)]
    pub repository: Option<RepositoryInfo>,
}

/// What the application knows about a source control working directory,
/// read from the checkout itself rather than guessed from its name.
///
/// Per decision D10 this describes; it never drives. Nothing here runs a
/// source control command.
///
/// This is what a *listing* can afford for every row: a few small reads per
/// folder. Whether the working tree has uncommitted changes costs a pass
/// over every tracked file, so it is answered for the selected repository
/// only, in that repository's own view data, and is not on the wire here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInfo {
    /// The host the checkout came from - `github.com`, `gitlab.com`,
    /// `bitbucket.org`, `dev.azure.com`, or whatever else its remote names.
    /// `None` for a checkout with no remote configured.
    pub provider: Option<String>,
    /// The branch checked out, or `None` when the working copy is not on a
    /// branch - a detached head, mid-rebase, or a fresh clone with no
    /// commits yet.
    pub branch: Option<String>,
    /// The address the checkout tracks, as written in its own
    /// configuration.
    pub remote: Option<String>,
}

/// One view of a path, and the plugin that should present it.
///
/// Named separately from [`Response::FileView`]'s own fields because a
/// path can carry several of these, and a list of pairs is clearer on the
/// wire than parallel lists of names and values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginView {
    /// Name of the plugin that produced `data`.
    pub plugin: String,
    /// The plugin's view data, ready for its presentation half.
    pub data: serde_json::Value,
}

/// A response sent from the service back to a front end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Response {
    /// The requested directory's immediate entries.
    Directory {
        /// Entries in the directory, sorted by name.
        entries: Vec<DirectoryEntry>,
    },
    /// A file's content, as produced by the plugin that recognised it.
    FileView {
        /// Name of the plugin that produced `data`.
        plugin: String,
        /// The plugin's view data, ready for its presentation half.
        data: serde_json::Value,
        /// Further views of the same path, each to be presented by the
        /// plugin it names and appended below the first.
        ///
        /// Always empty for a file. A file has exactly one type - two
        /// plugins claiming one file is a defect, which is why the
        /// extension tiebreak exists. A folder is several things at once
        /// and honestly so: a source control working copy that is also a
        /// Cargo workspace has two true descriptions, and a reader wants
        /// both.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        also: Vec<PluginView>,
    },
    /// The request could not be completed.
    Error {
        /// A human-readable description of the failure.
        message: String,
    },
    /// The Repos Directory roots this machine has configured.
    ReposRoots {
        /// Every stored root, in the order they were added.
        roots: Vec<ReposRoot>,
        /// The platform's default, offered on a first run: `Z:\repos` on
        /// Windows, `~/repos` elsewhere. Present whether or not anything is
        /// configured, so a front end can offer it without knowing the rule.
        default: String,
    },
    /// An operation (rename, copy, delete, extract) completed successfully.
    Done,
}

/// One configured Repos Directory.
///
/// Per decision D9 exactly one root is active at a time, but they are stored
/// as a list so several can be supported later without moving anybody's
/// settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReposRoot {
    /// The directory itself.
    pub path: String,
    /// Whether this is the root the application opens at.
    pub active: bool,
}

/// Resolves [`SOCKET_NAME`] to a platform-appropriate local socket name,
/// preferring a namespaced name and falling back to a filesystem path where
/// namespaced sockets are not supported.
///
/// # Errors
/// Returns an error if the resolved name is not valid on this platform.
pub fn socket_name() -> io::Result<Name<'static>> {
    let chosen: &'static str = CHOSEN_SOCKET_NAME.get_or_init(|| SOCKET_NAME.to_owned());
    if GenericNamespaced::is_supported() {
        chosen.to_ns_name::<GenericNamespaced>()
    } else {
        std::env::temp_dir()
            .join(chosen)
            .to_fs_name::<GenericFilePath>()
    }
}

/// The socket this process uses, settled the first time anything asks.
static CHOSEN_SOCKET_NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Makes this process use a socket of its own instead of the shared one.
///
/// For tests that start a service in-process. The socket name used to be
/// fixed, and a test harness that could not bind it - because another test
/// binary, or the reader's own running Repos Explorer, already held it -
/// quietly connected to whatever was there instead. So test suites carried
/// out real file operations and undos through a service they had not
/// started: sharing one undo journal between binaries running at once,
/// which is where intermittent undo failures came from, and replacing the
/// journal of the reader's live service whenever one was up.
///
/// Returns `false` if the name was already settled, by an earlier call or
/// by [`socket_name`] having been used, so a harness that asks too late
/// finds out rather than silently sharing.
#[must_use]
pub fn use_private_socket(name: String) -> bool {
    CHOSEN_SOCKET_NAME.set(name).is_ok()
}

/// Reads one length-prefixed, JSON-encoded message from `reader`.
///
/// # Errors
/// Returns an error if the announced length exceeds [`MAX_MESSAGE_BYTES`],
/// if the underlying I/O fails, or if the bytes read are not a valid `T`.
pub fn read_message<T: serde::de::DeserializeOwned, R: Read>(mut reader: R) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_be_bytes(len_bytes);
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message of {len} bytes exceeds the {MAX_MESSAGE_BYTES} byte limit"),
        ));
    }
    let len = len as usize;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

/// How deeply the encoded JSON in `bytes` nests.
///
/// Counted over the bytes rather than over the value, so it measures the
/// same thing the reader will measure - the envelope included - and costs
/// one pass with no allocation.
fn json_depth(bytes: &[u8]) -> usize {
    let mut depth = 0usize;
    let mut deepest = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Writes one length-prefixed, JSON-encoded message to `writer`.
///
/// # Errors
/// Returns an error if `value` cannot be encoded or the underlying I/O fails.
pub fn write_message<T: Serialize, W: Write>(mut writer: W, value: &T) -> io::Result<()> {
    let buf =
        serde_json::to_vec(value).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    let depth = json_depth(&buf);
    if depth > MAX_MESSAGE_DEPTH {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "message nested {depth} deep exceeds the {MAX_MESSAGE_DEPTH} level limit \
                 the reader will accept"
            ),
        ));
    }
    let len =
        u32::try_from(buf.len()).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if len > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("message of {len} bytes exceeds the {MAX_MESSAGE_BYTES} byte limit"),
        ));
    }
    writer.write_all(&len.to_be_bytes())?;
    writer.write_all(&buf)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::{
        DirectoryEntry, MAX_MESSAGE_BYTES, PluginView, ReposRoot, RepositoryInfo, Request,
        Response, VERSION, read_message, socket_name, write_message,
    };
    use std::io::{self, Read, Write};

    #[test]
    fn refuses_a_length_prefix_larger_than_the_message_limit() {
        let mut frame = (MAX_MESSAGE_BYTES + 1).to_be_bytes().to_vec();
        frame.extend_from_slice(b"the payload is never read");

        let err = read_message::<Response, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn round_trips_a_response_through_the_wire_format() {
        let response = Response::Directory {
            entries: vec![DirectoryEntry {
                name: "src".to_owned(),
                is_dir: true,
                size: 4096,
                modified: Some(1_700_000_000),
                repository: None,
            }],
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &response).unwrap();

        let decoded: Response = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn round_trips_a_file_view_through_the_wire_format() {
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "hello", "truncated": false }),
            also: Vec::new(),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &response).unwrap();

        let decoded: Response = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn round_trips_a_create_directory_request_through_the_wire_format() {
        let request = Request::CreateDirectory {
            path: "new_dir".to_owned(),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();

        let decoded: Request = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn round_trips_a_create_file_request_through_the_wire_format() {
        let request = Request::CreateFile {
            path: "new_file.txt".to_owned(),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();

        let decoded: Request = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn a_listing_carries_what_each_folder_is_as_a_working_copy() {
        let entries = vec![
            DirectoryEntry {
                name: "explorer".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: Some("github.com".to_owned()),
                    branch: Some("main".to_owned()),
                    remote: Some("https://github.com/owner/explorer.git".to_owned()),
                }),
            },
            DirectoryEntry {
                name: "scratch".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: None,
            },
        ];

        let mut buffer = Vec::new();
        write_message(&mut buffer, &Response::Directory { entries }).unwrap();
        let read: Response = read_message(&mut buffer.as_slice()).unwrap();

        match read {
            Response::Directory { entries } => {
                let repository = entries[0].repository.as_ref().expect("a working copy");
                assert_eq!(repository.provider.as_deref(), Some("github.com"));
                assert_eq!(repository.branch.as_deref(), Some("main"));
                assert!(entries[1].repository.is_none(), "an ordinary folder");
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn a_listing_from_an_older_service_still_reads() {
        // `repository` is defaulted rather than required, so a front end
        // built after this change can still read a reply from a service
        // built before it.
        let older =
            r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0,"modified":null}]}}"#;
        let response: Response = serde_json::from_str(older).unwrap();

        match response {
            Response::Directory { entries } => assert!(entries[0].repository.is_none()),
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn the_roots_reply_round_trips() {
        let sent = Response::ReposRoots {
            roots: vec![
                ReposRoot {
                    path: "/home/ada/repos".to_owned(),
                    active: true,
                },
                ReposRoot {
                    path: "/mnt/work/repos".to_owned(),
                    active: false,
                },
            ],
            default: "/home/ada/repos".to_owned(),
        };

        let mut buffer = Vec::new();
        write_message(&mut buffer, &sent).unwrap();
        let read: Response = read_message(&mut buffer.as_slice()).unwrap();

        assert_eq!(read, sent);
        match read {
            Response::ReposRoots { roots, .. } => assert_eq!(
                roots.iter().filter(|root| root.active).count(),
                1,
                "exactly one root is active (decision D9)"
            ),
            other => panic!("expected roots, got {other:?}"),
        }
    }

    /// A reader that hands back one byte per call, so a framing routine that
    /// assumes a single `read` fills its buffer is caught.
    struct OneByteAtATime<'a> {
        bytes: &'a [u8],
    }

    impl Read for OneByteAtATime<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() || self.bytes.is_empty() {
                return Ok(0);
            }
            buf[0] = self.bytes[0];
            self.bytes = &self.bytes[1..];
            Ok(1)
        }
    }

    /// A reader that records how much of the stream was actually consumed.
    struct CountingReader<'a> {
        bytes: &'a [u8],
        consumed: usize,
    }

    impl Read for CountingReader<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let read = self.bytes.read(buf)?;
            self.consumed += read;
            Ok(read)
        }
    }

    /// A writer that accepts the four-byte prefix and then reports the peer
    /// has gone, the way a closed socket does mid-message.
    struct HangsUpAfterTheLengthPrefix {
        accepted: usize,
    }

    impl Write for HangsUpAfterTheLengthPrefix {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.accepted >= 4 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "the peer hung up",
                ));
            }
            self.accepted += buf.len();
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A JSON array nested `depth` deep around a single string.
    fn nested_value(depth: usize) -> serde_json::Value {
        let mut value = serde_json::Value::String("leaf".to_owned());
        for _ in 0..depth {
            value = serde_json::Value::Array(vec![value]);
        }
        value
    }

    fn frame_of<T: serde::Serialize>(value: &T) -> Vec<u8> {
        let mut buffer = Vec::new();
        write_message(&mut buffer, value).expect("the message encodes");
        buffer
    }

    #[test]
    fn a_length_prefix_cut_short_ends_the_read_rather_than_guessing_a_length() {
        let err = read_message::<Request, _>([0u8, 0, 12].as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_length_prefix_split_across_reads_is_reassembled() {
        let frame = frame_of(&Request::ListDirectory {
            path: "/home/ada/repos".to_owned(),
        });

        let decoded: Request = read_message(OneByteAtATime {
            bytes: frame.as_slice(),
        })
        .expect("a dribbling reader still yields one whole message");

        assert_eq!(
            decoded,
            Request::ListDirectory {
                path: "/home/ada/repos".to_owned()
            }
        );
    }

    #[test]
    fn a_length_of_zero_is_an_error_rather_than_an_empty_message() {
        let frame = [0u8, 0, 0, 0];

        let err = read_message::<Request, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_length_of_exactly_the_message_limit_is_inside_the_cap() {
        // The cap is a maximum, not an exclusive bound: a frame announcing
        // exactly MAX_MESSAGE_BYTES must be read, so it fails for running out
        // of payload rather than for being too large.
        let mut frame = MAX_MESSAGE_BYTES.to_be_bytes().to_vec();
        frame.extend_from_slice(b"\"Undo\"");

        let err = read_message::<Request, _>(frame.as_slice()).unwrap_err();

        assert_eq!(
            err.kind(),
            io::ErrorKind::UnexpectedEof,
            "the limit itself is accepted, then the short payload ends the read"
        );
    }

    #[test]
    fn a_length_over_the_limit_is_refused_before_a_byte_of_payload_is_touched() {
        // Four bytes must not be able to make the reader commit to 4 GiB, so
        // the refusal has to happen off the prefix alone.
        let mut frame = (MAX_MESSAGE_BYTES + 1).to_be_bytes().to_vec();
        frame.extend_from_slice(&vec![b'x'; 4096]);
        let mut reader = CountingReader {
            bytes: frame.as_slice(),
            consumed: 0,
        };

        let err = read_message::<Request, _>(&mut reader).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            reader.consumed, 4,
            "only the length prefix is read; the payload is never pulled in"
        );
    }

    #[test]
    fn the_largest_possible_length_prefix_is_refused_rather_than_allocated() {
        let mut frame = u32::MAX.to_be_bytes().to_vec();
        frame.extend_from_slice(b"nothing like four gibibytes follows");
        let mut reader = CountingReader {
            bytes: frame.as_slice(),
            consumed: 0,
        };

        let err = read_message::<Response, _>(&mut reader).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(reader.consumed, 4);
    }

    #[test]
    fn a_payload_shorter_than_its_prefix_claims_ends_the_read() {
        let mut frame = 4096u32.to_be_bytes().to_vec();
        frame.extend_from_slice(b"\"Undo\"");

        let err = read_message::<Request, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn a_stream_that_stops_mid_payload_ends_the_read() {
        let mut frame = frame_of(&Request::ListDirectory {
            path: "/home/ada/repos".to_owned(),
        });
        frame.truncate(frame.len() - 3);

        let err = read_message::<Request, _>(OneByteAtATime {
            bytes: frame.as_slice(),
        })
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn reading_one_message_leaves_the_next_one_in_the_stream() {
        // The service reads request after request from one connection, so a
        // read that over-ran its own frame would eat the following message.
        let mut stream = frame_of(&Request::Undo);
        stream.extend_from_slice(&frame_of(&Request::CreateFile {
            path: "notes.md".to_owned(),
        }));
        let mut reader = stream.as_slice();

        let first: Request = read_message(&mut reader).expect("the first message");
        let second: Request = read_message(&mut reader).expect("the second message");

        assert_eq!(first, Request::Undo);
        assert_eq!(
            second,
            Request::CreateFile {
                path: "notes.md".to_owned()
            }
        );
        assert!(reader.is_empty(), "both frames are consumed exactly");
    }

    #[test]
    fn a_payload_that_is_not_utf8_is_an_error_rather_than_a_panic() {
        let payload = [0xffu8, 0xfe, 0x00, 0x80];
        let mut frame = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        frame.extend_from_slice(&payload);

        let err = read_message::<Request, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_payload_naming_a_variant_this_build_does_not_know_is_an_error() {
        let payload = br#"{"Teleport":{"path":"/somewhere"}}"#;
        let mut frame = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
        frame.extend_from_slice(payload);

        let err = read_message::<Request, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_write_that_fails_after_the_prefix_surfaces_the_io_error() {
        let mut writer = HangsUpAfterTheLengthPrefix { accepted: 0 };

        let err = write_message(
            &mut writer,
            &Request::ListDirectory {
                path: "/home/ada/repos".to_owned(),
            },
        )
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn every_request_variant_survives_the_wire_unchanged() {
        let requests = vec![
            Request::ListDirectory {
                path: String::new(),
            },
            Request::ViewFile {
                path: "a.txt".to_owned(),
            },
            Request::Open {
                path: ".".to_owned(),
            },
            Request::Rename { items: Vec::new() },
            Request::Rename {
                items: vec![
                    ("old".to_owned(), "new".to_owned()),
                    (String::new(), String::new()),
                ],
            },
            Request::Copy {
                items: vec![("from".to_owned(), "to".to_owned())],
            },
            Request::Copy { items: Vec::new() },
            Request::Delete { paths: Vec::new() },
            Request::Delete {
                paths: vec!["one".to_owned(), String::new()],
            },
            Request::Extract {
                archive: "a.zip".to_owned(),
                destination: String::new(),
            },
            Request::CreateDirectory {
                path: "d".to_owned(),
            },
            Request::ReposRoots,
            Request::SetReposRoot {
                path: "Z:\\repos".to_owned(),
            },
            Request::WriteFile {
                path: "f".to_owned(),
                content: String::new(),
            },
            Request::Undo,
            Request::CreateFile {
                path: "f".to_owned(),
            },
        ];

        for request in requests {
            let frame = frame_of(&request);
            let decoded: Request = read_message(frame.as_slice()).expect("the frame decodes");
            assert_eq!(decoded, request, "{request:?} did not survive the wire");
        }
    }

    #[test]
    fn every_response_variant_survives_the_wire_unchanged() {
        let responses = vec![
            Response::Directory {
                entries: Vec::new(),
            },
            Response::Directory {
                entries: vec![DirectoryEntry {
                    name: String::new(),
                    is_dir: false,
                    size: u64::MAX,
                    modified: Some(0),
                    repository: Some(RepositoryInfo::default()),
                }],
            },
            Response::FileView {
                plugin: String::new(),
                data: serde_json::Value::Null,
                also: Vec::new(),
            },
            Response::FileView {
                plugin: "folder".to_owned(),
                data: serde_json::json!({ "kind": "folder" }),
                also: vec![
                    PluginView {
                        plugin: "project-cargo".to_owned(),
                        data: serde_json::json!({ "members": [] }),
                    },
                    PluginView {
                        plugin: String::new(),
                        data: serde_json::Value::Bool(false),
                    },
                ],
            },
            Response::Error {
                message: String::new(),
            },
            Response::ReposRoots {
                roots: Vec::new(),
                default: String::new(),
            },
            Response::Done,
        ];

        for response in responses {
            let frame = frame_of(&response);
            let decoded: Response = read_message(frame.as_slice()).expect("the frame decodes");
            assert_eq!(decoded, response, "{response:?} did not survive the wire");
        }
    }

    #[test]
    fn a_path_of_awkward_characters_survives_the_wire_unchanged() {
        // Paths are whatever the platform allows: quotes and backslashes that
        // JSON must escape, control characters, and text outside the basic
        // multilingual plane.
        let awkward =
            "Z:\\repos\\\"quoted\"\\line\nbreak\\tab\there\\nul\u{0}\\\u{1f600}\\\u{202e}rtl";
        let request = Request::SetReposRoot {
            path: awkward.to_owned(),
        };

        let frame = frame_of(&request);
        let decoded: Request = read_message(frame.as_slice()).expect("the frame decodes");

        assert_eq!(decoded, request);
    }

    #[test]
    fn an_entry_with_no_modified_time_keeps_that_absence_across_the_wire() {
        // `None` has to arrive as `None`: a filesystem that reports no
        // timestamp must not come out the other side as the epoch.
        let response = Response::Directory {
            entries: vec![DirectoryEntry {
                name: "src".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: None,
                    branch: Some("main".to_owned()),
                    remote: None,
                }),
            }],
        };

        let frame = frame_of(&response);
        let decoded: Response = read_message(frame.as_slice()).expect("the frame decodes");

        match decoded {
            Response::Directory { entries } => {
                assert!(entries[0].modified.is_none());
                let repository = entries[0].repository.as_ref().expect("a working copy");
                assert!(repository.provider.is_none());
                assert!(repository.remote.is_none());
                assert_eq!(repository.branch.as_deref(), Some("main"));
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn a_deeply_nested_file_view_survives_the_wire_unchanged() {
        let response = Response::FileView {
            plugin: "json".to_owned(),
            data: serde_json::json!({ "root": nested_value(60) }),
            also: Vec::new(),
        };

        let frame = frame_of(&response);
        let decoded: Response = read_message(frame.as_slice()).expect("the frame decodes");

        assert_eq!(decoded, response);
    }

    /// Hostile input must not take the stack with it.
    ///
    /// The frame is built by hand rather than with `frame_of`, because
    /// `write_message` now refuses to produce one this deep. That guard
    /// protects this crate from itself; it says nothing about what someone
    /// else can put on the socket, and the reader still has to answer for
    /// that on its own.
    #[test]
    fn a_file_view_nested_past_the_decoder_limit_is_refused_rather_than_overflowing() {
        let response = Response::FileView {
            plugin: "json".to_owned(),
            data: nested_value(512),
            also: Vec::new(),
        };
        let payload = serde_json::to_vec(&response).expect("a value always encodes");
        let mut frame = u32::try_from(payload.len())
            .expect("a small message")
            .to_be_bytes()
            .to_vec();
        frame.extend_from_slice(&payload);

        let err = read_message::<Response, _>(frame.as_slice()).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_file_view_from_an_older_service_without_the_also_list_still_reads() {
        // `also` arrived with the folder plugins; a reply written before it
        // has no such field, and must still be a file view rather than an
        // error.
        let older = r#"{"FileView":{"plugin":"text","data":{"content":"hi"}}}"#;

        let response: Response = serde_json::from_str(older).expect("an older reply still reads");

        match response {
            Response::FileView { also, plugin, .. } => {
                assert_eq!(plugin, "text");
                assert!(also.is_empty());
            }
            other => panic!("expected a file view, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_also_list_is_left_off_the_wire_entirely() {
        // The field is skipped when empty, which is what lets an older front
        // end read a newer service's reply to a plain file.
        let frame = frame_of(&Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::Value::Null,
            also: Vec::new(),
        });
        let json = String::from_utf8(frame[4..].to_vec()).expect("the payload is text");

        assert!(!json.contains("also"), "{json} still carries an empty list");
    }

    #[test]
    fn an_entry_missing_both_of_its_optional_fields_still_reads() {
        let older = r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0}]}}"#;

        let response: Response = serde_json::from_str(older).expect("an older entry still reads");

        match response {
            Response::Directory { entries } => {
                assert!(entries[0].modified.is_none());
                assert!(entries[0].repository.is_none());
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn an_entry_carrying_a_field_this_build_does_not_know_still_reads() {
        // The other direction of the same compatibility promise: a front end
        // built before a field was added must not choke on a newer service.
        let newer = r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0,"modified":null,"ahead_by":3}]}}"#;

        let response: Response = serde_json::from_str(newer).expect("a newer entry still reads");

        match response {
            Response::Directory { entries } => assert_eq!(entries[0].name, "src"),
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn nothing_on_the_wire_carries_the_protocol_version() {
        // VERSION is 2, but no frame names it and nothing negotiates it, so a
        // peer speaking another version is indistinguishable from this one and
        // a mismatch cannot be detected. Whoever adds a handshake should find
        // this test failing.
        let frame = frame_of(&Request::Undo);
        let json = String::from_utf8(frame[4..].to_vec()).expect("the payload is text");
        assert_eq!(json, "\"Undo\"");
        assert!(
            !json.contains(&VERSION.to_string()),
            "{json} names the protocol version"
        );

        let from_another_version: Request =
            read_message(frame.as_slice()).expect("accepted without a version check");
        assert_eq!(from_another_version, Request::Undo);
    }

    /// What this build writes, its peer can read - at every depth.
    ///
    /// The two used to disagree: `write_message` accepted any nesting while
    /// `read_message` stopped at the decoder's recursion limit, so a view
    /// 126 levels deep was written to the socket and then refused by the
    /// front end as invalid data. The JSON plugin puts a whole parsed
    /// document into its view data and its own parse allows more nesting
    /// than the envelope leaves room for, so a file in that band was
    /// unreadable for no reason the reader could see.
    ///
    /// The rule now is the one this test's name always claimed: a message
    /// is either refused before it is written, or it reads back. Never
    /// written-and-unreadable.
    #[test]
    fn a_file_view_this_build_writes_can_be_read_back_by_its_peer() {
        for depth in [0usize, 1, 60, 120, 124, 125, 126, 130, 200] {
            let response = Response::FileView {
                plugin: "json".to_owned(),
                data: nested_value(depth),
                also: Vec::new(),
            };

            let mut wire = Vec::new();
            if let Err(err) = write_message(&mut wire, &response) {
                assert_eq!(err.kind(), io::ErrorKind::InvalidData);
                assert!(
                    wire.is_empty(),
                    "a refused message must leave nothing on the wire, or the                      peer reads a prefix with no message after it"
                );
            } else {
                let read: Response = read_message(wire.as_slice()).unwrap_or_else(|err| {
                    panic!("{depth} levels were written and cannot be read: {err}")
                });
                assert_eq!(read, response);
            }
        }
    }

    #[test]
    fn the_socket_name_is_valid_on_this_platform() {
        socket_name().expect("SOCKET_NAME resolves on the platform this test runs on");
    }

    /// The other side of the same rule: what the writer refuses, it
    /// refuses out loud, naming depth rather than failing as a generic
    /// encoding error.
    #[test]
    fn a_message_too_deep_for_the_reader_is_refused_by_the_writer() {
        let mut value = serde_json::Value::Null;
        for _ in 0..200 {
            value = serde_json::Value::Array(vec![value]);
        }
        let response = Response::FileView {
            plugin: "json".to_owned(),
            data: value,
            also: Vec::new(),
        };

        let mut out = Vec::new();
        let err = write_message(&mut out, &response).expect_err("too deep to travel");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("nested"),
            "the refusal has to say what is wrong with it: {err}"
        );
        assert!(
            out.is_empty(),
            "and nothing may go on the wire, or the peer reads a prefix with no message"
        );
    }

    /// The boundary itself, so a later change to either side cannot drift
    /// them apart without a test saying so.
    #[test]
    fn the_writers_limit_is_the_readers_limit() {
        for depth in [100usize, 120, 125] {
            let mut value = serde_json::Value::Null;
            for _ in 0..depth {
                value = serde_json::Value::Array(vec![value]);
            }
            let response = Response::FileView {
                plugin: "json".to_owned(),
                data: value,
                also: Vec::new(),
            };

            let mut wire = Vec::new();
            write_message(&mut wire, &response)
                .unwrap_or_else(|err| panic!("{depth} levels should travel: {err}"));
            let read: Response = read_message(wire.as_slice())
                .unwrap_or_else(|err| panic!("{depth} levels should be readable: {err}"));

            assert_eq!(read, response, "what went out is what comes back");
        }
    }

    /// A brace inside a string is text, not nesting - otherwise a file
    /// full of braces would be refused for a depth it does not have.
    #[test]
    fn braces_inside_a_string_do_not_count_as_nesting() {
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": "{[{[".repeat(200) }),
            also: Vec::new(),
        };

        let mut wire = Vec::new();
        write_message(&mut wire, &response).expect("braces in text are not nesting");
        let read: Response = read_message(wire.as_slice()).expect("and it reads back");

        assert_eq!(read, response);
    }

    /// An escaped quote does not end the string it is in, so the braces
    /// after it are still text.
    #[test]
    fn an_escaped_quote_does_not_end_the_string_it_is_in() {
        let response = Response::FileView {
            plugin: "text".to_owned(),
            data: serde_json::json!({ "content": format!("\\\"{}", "{".repeat(200)) }),
            also: Vec::new(),
        };

        let mut wire = Vec::new();
        write_message(&mut wire, &response).expect("still text, however it is quoted");
        let read: Response = read_message(wire.as_slice()).expect("and it reads back");

        assert_eq!(read, response);
    }

    /// Choosing a private socket after the name has been settled fails out
    /// loud. The name is settled by whichever comes first in a process, and
    /// a harness that asked too late would otherwise go on sharing the
    /// socket it was trying to leave - which is the fault this exists to
    /// end. Deterministic whatever order the tests run in: once
    /// `socket_name` has been called here, the choice is made.
    #[test]
    fn a_private_socket_chosen_after_the_name_is_settled_is_refused() {
        socket_name().expect("the platform has a socket name");

        assert!(!super::use_private_socket("too-late.sock".to_owned()));
    }
}
