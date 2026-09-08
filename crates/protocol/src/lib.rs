//! IPC message types, versioned, shared by the service and both front ends.

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Name, NameType, ToFsName, ToNsName,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};

/// The protocol version this build speaks. Bump whenever [`Request`] or
/// [`Response`] changes shape in a way that is not backward compatible.
pub const VERSION: u32 = 1;

/// The name other processes use to find the service's local socket.
pub const SOCKET_NAME: &str = "reposphereexplorer.sock";

/// The largest message [`read_message`] will accept, in bytes. The length
/// prefix arrives before any of the payload it describes, so without a cap
/// four bytes are enough to make the reader commit up to 4 GiB. Real
/// messages are far smaller - a file view is capped at 64 KiB by its
/// plugin, and the largest thing on the wire is a directory listing.
pub const MAX_MESSAGE_BYTES: u32 = 64 * 1024 * 1024;

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
    /// Renames (or moves) `from` to `to`. Journaled.
    Rename {
        /// The existing path.
        from: String,
        /// The path it should have afterwards.
        to: String,
    },
    /// Copies the file at `from` to `to`. Journaled.
    Copy {
        /// The source file.
        from: String,
        /// The destination path.
        to: String,
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
    },
    /// The request could not be completed.
    Error {
        /// A human-readable description of the failure.
        message: String,
    },
    /// An operation (rename, copy, delete, extract) completed successfully.
    Done,
}

/// Resolves [`SOCKET_NAME`] to a platform-appropriate local socket name,
/// preferring a namespaced name and falling back to a filesystem path where
/// namespaced sockets are not supported.
///
/// # Errors
/// Returns an error if the resolved name is not valid on this platform.
pub fn socket_name() -> io::Result<Name<'static>> {
    if GenericNamespaced::is_supported() {
        SOCKET_NAME.to_ns_name::<GenericNamespaced>()
    } else {
        std::env::temp_dir()
            .join(SOCKET_NAME)
            .to_fs_name::<GenericFilePath>()
    }
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

/// Writes one length-prefixed, JSON-encoded message to `writer`.
///
/// # Errors
/// Returns an error if `value` cannot be encoded or the underlying I/O fails.
pub fn write_message<T: Serialize, W: Write>(mut writer: W, value: &T) -> io::Result<()> {
    let buf =
        serde_json::to_vec(value).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
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
        DirectoryEntry, MAX_MESSAGE_BYTES, Request, Response, read_message, write_message,
    };

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
}
