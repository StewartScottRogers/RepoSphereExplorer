//! IPC message types, versioned, shared by the service and both front ends.

use interprocess::local_socket::{
    GenericFilePath, GenericNamespaced, Name, NameType, ToFsName, ToNsName,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::Path;

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
    /// Finds every file and folder in the active Repos Directory whose name
    /// contains `query`, ignoring case. Walks what `git` would: files a
    /// `.gitignore`, `.ignore` or global exclude rules out are skipped, and
    /// nothing under `.git` is ever returned.
    ///
    /// The reply is [`Response::Names`]. An empty or whitespace-only query
    /// finds nothing rather than everything.
    FindNames {
        /// Part of a file or folder name.
        query: String,
        /// The most matches to return; the reply says if there were more.
        limit: usize,
    },
    /// Whether the working copy at `path` has uncommitted changes to the
    /// files it tracks.
    ///
    /// A pass over every tracked file of one checkout, which is why a
    /// listing does not carry it: a front end asks for the repository rows
    /// it has on screen, one request each, after the listing has landed.
    /// The reply is [`Response::WorkingTree`].
    WorkingTreeStatus {
        /// The working copy's own folder.
        path: String,
    },
    /// The working copies found up to three folder levels below `root`
    /// (#591), a background scan the service keeps for the session: later
    /// polls with the same `root` return the same scan's latest progress,
    /// until [`Response::AllRepositories`] says it is done. `refresh`
    /// discards that scan and starts again - what F5 (Refresh) does while
    /// the view is open.
    ///
    /// The reply is [`Response::AllRepositories`].
    AllRepositories {
        /// The Repos Directory to scan below.
        root: String,
        /// Whether to discard any scan already under way, or cached from
        /// one that finished, and start a new one.
        refresh: bool,
    },
    /// Finds every certificate, certificate signing request and private key
    /// committed under the active Repos Directory (#621) - read-only (D10,
    /// rule 8): nothing here issues, renews, revokes or deploys.
    ///
    /// Walked with the same rules [`Request::FindNames`] uses.
    ///
    /// The reply is [`Response::Certificates`].
    FindCertificates,
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
    /// Whether this is an ordinary clone, a linked worktree, or a submodule
    /// (#587).
    #[serde(default)]
    pub kind: RepositoryKind,
    /// The latest modification time among the checkout's `index`, `HEAD` and
    /// `logs/HEAD`, in seconds since `UNIX_EPOCH` - the working copy's own
    /// "last activity", which changes on checkout, commit, staging and
    /// branch switches, unlike the folder's own modification time (#588).
    /// Falls back to the folder's own time when none of the three exist.
    #[serde(default)]
    pub last_activity: Option<u64>,
    /// `FETCH_HEAD`'s own modification time, in seconds since `UNIX_EPOCH` -
    /// when the checkout (or, for a worktree, the clone it shares) was last
    /// fetched, or `None` when it never has (#589). A single stat, like
    /// `last_activity`, never a directory read.
    #[serde(default)]
    pub last_fetch: Option<u64>,
}

/// What kind of working copy a [`RepositoryInfo`] describes. Mirrors
/// `plugin_directory::repository::Kind`, duplicated here rather than
/// shared: this crate carries the wire format for both front ends, and
/// takes no dependency on a plugin crate to describe it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryKind {
    /// An ordinary checkout.
    #[default]
    Clone,
    /// A linked worktree (`git worktree add`), sharing a clone's git
    /// directory.
    Worktree {
        /// The clone's working directory, as an absolute path.
        clone: String,
        /// Whether `clone` still exists.
        clone_exists: bool,
    },
    /// A submodule, pinned by an outer working copy.
    Submodule {
        /// The outer working copy's directory, as an absolute path.
        outer: String,
    },
}

/// One working copy found by [`Request::AllRepositories`] (#591).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllRepositoryEntry {
    /// The repository folder's own name.
    pub name: String,
    /// Its parent folder's path relative to the Repos Directory, `/`-joined
    /// on every platform, empty for a direct child of the root.
    pub location: String,
    /// What the application knows about it, read the same way a listing
    /// row's is.
    pub repository: RepositoryInfo,
}

/// One file or folder found by [`Request::FindNames`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameMatch {
    /// Its path relative to the Repos Directory, with `/` between the
    /// components on every platform.
    pub path: String,
    /// Whether it is itself a directory.
    pub is_dir: bool,
    /// The nearest working copy holding it - the match itself when it is
    /// one - as a path relative to the Repos Directory, `/`-separated, and
    /// empty when that working copy is the Repos Directory itself. `None`
    /// when no working copy holds it.
    pub repository: Option<String>,
}

/// One certificate's fields, as [`Request::FindCertificates`] reports them.
///
/// Mirrors `plugin_certificate::CertificateSummary`, duplicated here rather
/// than shared: this crate carries the wire format for both front ends and
/// takes no dependency on a plugin crate to describe it (see
/// [`RepositoryKind`]'s own note on the same choice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateSummary {
    /// The certificate subject's distinguished name.
    pub subject: String,
    /// The issuing certificate authority's distinguished name.
    pub issuer: String,
    /// The certificate's serial number, as colon-separated hex.
    pub serial: String,
    /// Start of the certificate's validity period, Unix seconds.
    pub not_before: i64,
    /// End of the certificate's validity period, Unix seconds.
    pub not_after: i64,
    /// Whether the certificate's subject and issuer distinguished names are
    /// identical - a description read off the certificate, not a
    /// cryptographic signature check (D10, rule 8: detect, do not drive).
    pub self_signed: bool,
}

/// One PEM block within a [`CertificateFindingKind::Blocks`] list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateBlock {
    /// A parsed certificate.
    Certificate(CertificateSummary),
    /// A private key - present, and nothing more: no response carries key
    /// material.
    PrivateKey,
    /// A certificate signing request - present, and nothing more.
    CertificateRequest,
    /// A `CERTIFICATE`-labelled block whose content did not decode as
    /// X.509, reported rather than dropped.
    Unreadable,
}

/// What one PEM-encoded file held, found by [`Request::FindCertificates`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CertificateFindingKind {
    /// Every certificate, private key and certificate signing request block
    /// the file held, in file order.
    Blocks(Vec<CertificateBlock>),
    /// The file's extension claimed it, but nothing PEM-encoded could be
    /// read from it.
    Unreadable,
}

/// One certificate-bearing file found by [`Request::FindCertificates`]
/// (#621).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateFinding {
    /// The file's path relative to the Repos Directory, `/`-separated.
    pub path: String,
    /// The nearest working copy holding it, as [`NameMatch::repository`] is
    /// described.
    pub repository: Option<String>,
    /// What the file held.
    pub kind: CertificateFindingKind,
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
    /// The answer to [`Request::FindNames`].
    Names {
        /// The Repos Directory that was searched, which every match's path
        /// is relative to, so a front end opens the folder that was actually
        /// searched rather than guessing at one it has open.
        root: String,
        /// The matches, in the order the walk met them.
        matches: Vec<NameMatch>,
        /// Whether there were more matches than the request's limit, so
        /// `matches` is not everything.
        cut_short: bool,
    },
    /// The answer to [`Request::WorkingTreeStatus`].
    WorkingTree {
        /// The folder that was asked about, as the request named it.
        path: String,
        /// What its tracked files look like, or `None` when the folder is
        /// not a working copy or its index could not be read - which is
        /// "cannot tell", never "no changes".
        status: Option<WorkingTreeSummary>,
    },
    /// The answer to [`Request::AllRepositories`]: what the background scan
    /// has found so far.
    AllRepositories {
        /// The working copies found so far, in the order the scan met them.
        entries: Vec<AllRepositoryEntry>,
        /// Whether the scan has finished. While this is `false`, a later
        /// poll of the same root returns a longer (or equal) list.
        done: bool,
    },
    /// The answer to [`Request::FindCertificates`].
    Certificates {
        /// Every certificate-bearing file found, in the order the walk met
        /// them.
        certificates: Vec<CertificateFinding>,
        /// Whether the walk finished rather than being cut short. Always
        /// `true`: unlike [`Self::Names`]'s `cut_short`, nothing here
        /// imposes a limit.
        complete: bool,
    },
}

/// Whether a working copy has uncommitted changes to the files it tracks,
/// as [`Response::WorkingTree`] carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingTreeSummary {
    /// How many tracked files differ from what was last staged.
    pub changed: usize,
    /// Whether the service stopped before examining every tracked file, so
    /// that nothing changed means "nothing found", not "nothing".
    pub partial: bool,
    /// The same answer as a line for a reader: `3 tracked files changed`.
    pub summary: String,
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

/// Why the Repos Directory's own root could not be listed (#592, #680):
/// decided by looking at the path directly rather than at the service's
/// answer, which only ever sends a stringified `io::Error` with no way to
/// tell "does not exist" apart from "permission denied" without parsing
/// English out of it. Shared by both front ends (#680) so a reader sees the
/// same explanation whichever one they are looking at. Only ever computed
/// for the Repos Directory's own root - a subfolder that fails to list
/// keeps its front end's ordinary error handling instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootProblem {
    /// The path, or the drive it names, does not exist.
    NotThere {
        /// The likely reason.
        cause: NotThereCause,
    },
    /// The path exists but could not be read - permission denied, or any
    /// other input/output error. Carries the service's own message, which
    /// is the detail a front end shows.
    NotReadable {
        /// The service's own error message.
        message: String,
    },
}

impl RootProblem {
    /// The title a front end draws in place of `root`'s listing: shared so
    /// the two front ends never disagree about what a reader is told for
    /// the same cause.
    #[must_use]
    pub fn title(&self, root: &Path) -> String {
        let path = root.display();
        match self {
            Self::NotThere { .. } => format!("The Repos Directory {path} is not available"),
            Self::NotReadable { .. } => format!("Repos Explorer cannot read {path}"),
        }
    }

    /// The detail line under [`Self::title`]: the likely cause, or the read
    /// error's own message.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::NotThere { cause } => cause.describe(),
            Self::NotReadable { message } => message.clone(),
        }
    }
}

/// The likely reason a Repos Directory path is not there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotThereCause {
    /// The path names a drive letter (`Z:\repos`) whose drive itself is not
    /// connected - a mapped network drive before the VPN is up, or an
    /// external disk that is unplugged. Carries the drive, e.g. `"Z:"`.
    DriveNotConnected(String),
    /// The drive, if any, is there; the folder itself is not.
    FolderMissing,
}

impl NotThereCause {
    /// The second line under "is not available": the likely cause.
    fn describe(&self) -> String {
        match self {
            Self::DriveNotConnected(drive) => format!("Drive {drive} is not connected"),
            Self::FolderMissing => "The folder does not exist".to_owned(),
        }
    }
}

/// Whether `root` names a Windows drive letter (`Z:\repos`, or a
/// forward-slash spelling of the same thing), and if so, which one - `"Z:"`.
/// Read from the path's text rather than [`std::path::Component::Prefix`],
/// which only Windows' own path parser ever produces: this way the "drive
/// not connected" case is exercisable by a unit test on any host, the Linux
/// runner that gates every pull request included.
fn drive_letter(root: &Path) -> Option<String> {
    let text = root.to_string_lossy();
    let mut chars = text.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':')).then(|| format!("{letter}:"))
}

/// Classifies why `root` is not there: whether it names a drive that is
/// itself missing, or is an ordinary missing folder.
fn not_there_cause(root: &Path) -> NotThereCause {
    let Some(drive) = drive_letter(root) else {
        return NotThereCause::FolderMissing;
    };
    let mut drive_root = drive.clone();
    drive_root.push(std::path::MAIN_SEPARATOR);
    if std::fs::metadata(drive_root).is_err() {
        NotThereCause::DriveNotConnected(drive)
    } else {
        NotThereCause::FolderMissing
    }
}

/// Classifies why the Repos Directory's root listing failed, from the path
/// itself and the message the failed request already carried.
#[must_use]
pub fn classify_root_problem(root: &Path, message: &str) -> RootProblem {
    match std::fs::metadata(root) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => RootProblem::NotThere {
            cause: not_there_cause(root),
        },
        _ => RootProblem::NotReadable {
            message: message.to_owned(),
        },
    }
}

/// The title a front end draws when the Repos Directory lists fine but
/// holds nothing yet - shared with [`RootProblem::title`] so every case is
/// worded the same way in both front ends.
#[must_use]
pub fn empty_root_title(root: &Path) -> String {
    format!("{} has no repositories yet", root.display())
}

/// The detail line under [`empty_root_title`].
pub const EMPTY_ROOT_DETAIL: &str = "Working copies cloned into it will appear here.";

/// Resolves [`SOCKET_NAME`] to a platform-appropriate local socket name,
/// preferring a namespaced name and falling back to a filesystem path where
/// namespaced sockets are not supported. The session owner's identifier is
/// folded in (GUIDANCE.md §2.1.1), so two accounts on one machine each get
/// their own socket rather than racing for a fixed, machine-global name.
///
/// # Errors
/// Returns an error if the resolved name is not valid on this platform.
pub fn socket_name() -> io::Result<Name<'static>> {
    let chosen: &'static str =
        CHOSEN_SOCKET_NAME.get_or_init(|| named_for_user(&session_identifier()));
    if GenericNamespaced::is_supported() {
        chosen.to_ns_name::<GenericNamespaced>()
    } else {
        std::env::temp_dir()
            .join(chosen)
            .to_fs_name::<GenericFilePath>()
    }
}

/// [`SOCKET_NAME`] with `identifier` worked in ahead of the extension, so
/// it differs between accounts. A pure function over the identifier so the
/// difference is testable without two real user accounts.
fn named_for_user(identifier: &str) -> String {
    let stem = SOCKET_NAME
        .strip_suffix(".sock")
        .expect("SOCKET_NAME ends in .sock");
    format!("{stem}-{identifier}.sock")
}

/// An identifier for the account running this process, unique enough on one
/// machine to keep two users' sockets apart. Not itself a security
/// boundary - [`SOCKET_NAME`]'s permissions are - just a way to stop them
/// colliding.
fn session_identifier() -> String {
    #[cfg(unix)]
    {
        rustix::process::geteuid().as_raw().to_string()
    }
    #[cfg(not(unix))]
    {
        // Resolving a security identifier (SID) needs Win32 calls that this
        // workspace's `unsafe_code = "forbid"` rules out (see
        // `service::owner_identity`'s same note); the logon name from the
        // environment is enough to tell two accounts' sockets apart.
        std::env::var("USERNAME").unwrap_or_default()
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
        CertificateBlock, CertificateFinding, CertificateFindingKind, CertificateSummary,
        DirectoryEntry, EMPTY_ROOT_DETAIL, MAX_MESSAGE_BYTES, NameMatch, NotThereCause, PluginView,
        ReposRoot, RepositoryInfo, RepositoryKind, Request, Response, RootProblem, VERSION,
        WorkingTreeSummary, classify_root_problem, empty_root_title, named_for_user, read_message,
        socket_name, write_message,
    };
    use std::io::{self, Read, Write};
    use std::path::Path;

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
    fn round_trips_a_working_tree_status_through_the_wire_format() {
        let request = Request::WorkingTreeStatus {
            path: "/repos/alpha".to_owned(),
        };
        let response = Response::WorkingTree {
            path: "/repos/alpha".to_owned(),
            status: Some(WorkingTreeSummary {
                changed: 3,
                partial: false,
                summary: "3 tracked files changed".to_owned(),
            }),
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();
        write_message(&mut buf, &response).unwrap();

        let mut reader = buf.as_slice();
        let decoded: Request = read_message(&mut reader).unwrap();
        assert_eq!(decoded, request);
        let decoded: Response = read_message(&mut reader).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn round_trips_a_find_names_request_through_the_wire_format() {
        let request = Request::FindNames {
            query: "Cargo.toml".to_owned(),
            limit: 500,
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();

        let decoded: Request = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn round_trips_the_names_found_through_the_wire_format() {
        let response = Response::Names {
            root: "/repos".to_owned(),
            matches: vec![
                NameMatch {
                    path: "explorer/crates/service/Cargo.toml".to_owned(),
                    is_dir: false,
                    repository: Some("explorer".to_owned()),
                },
                NameMatch {
                    path: "scratch/docker-compose".to_owned(),
                    is_dir: true,
                    repository: None,
                },
            ],
            cut_short: true,
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &response).unwrap();

        let decoded: Response = read_message(buf.as_slice()).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn round_trips_the_certificates_found_through_the_wire_format() {
        let request = Request::FindCertificates;
        let response = Response::Certificates {
            certificates: vec![
                CertificateFinding {
                    path: "explorer/certs/chain.pem".to_owned(),
                    repository: Some("explorer".to_owned()),
                    kind: CertificateFindingKind::Blocks(vec![
                        CertificateBlock::Certificate(CertificateSummary {
                            subject: "CN=example.com".to_owned(),
                            issuer: "CN=Test Root CA".to_owned(),
                            serial: "01:02:03".to_owned(),
                            not_before: 1_700_000_000,
                            not_after: 1_800_000_000,
                            self_signed: false,
                        }),
                        CertificateBlock::PrivateKey,
                        CertificateBlock::CertificateRequest,
                        CertificateBlock::Unreadable,
                    ]),
                },
                CertificateFinding {
                    path: "scratch/broken.pem".to_owned(),
                    repository: None,
                    kind: CertificateFindingKind::Unreadable,
                },
            ],
            complete: true,
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &request).unwrap();
        write_message(&mut buf, &response).unwrap();

        let mut reader = buf.as_slice();
        let decoded: Request = read_message(&mut reader).unwrap();
        assert_eq!(decoded, request);
        let decoded: Response = read_message(&mut reader).unwrap();
        assert_eq!(decoded, response);
    }

    #[test]
    fn a_full_page_of_names_is_far_inside_the_message_limit() {
        // The front end asks for the first 500. Long, deeply nested paths
        // in a long-named checkout, to measure the worst of a real page.
        let path = format!("{}/Cargo.toml", "a-fairly-long-folder-name".repeat(8));
        let response = Response::Names {
            root: "/repos".to_owned(),
            matches: vec![
                NameMatch {
                    path,
                    is_dir: false,
                    repository: Some("a-long-repository-name".to_owned()),
                };
                500
            ],
            cut_short: true,
        };

        let mut buf = Vec::new();
        write_message(&mut buf, &response).unwrap();

        assert!(
            buf.len() < 256 * 1024,
            "500 matches took {} bytes",
            buf.len()
        );
        assert!(buf.len() < MAX_MESSAGE_BYTES as usize);
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
                    kind: RepositoryKind::Clone,
                    last_activity: None,
                    last_fetch: None,
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
    fn a_worktree_or_submodule_carries_its_kind_across_the_wire() {
        let entries = vec![
            DirectoryEntry {
                name: "linked".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: Some("github.com".to_owned()),
                    branch: Some("side".to_owned()),
                    remote: Some("https://github.com/owner/name.git".to_owned()),
                    kind: RepositoryKind::Worktree {
                        clone: "/repos/clone".to_owned(),
                        clone_exists: true,
                    },
                    last_activity: None,
                    last_fetch: None,
                }),
            },
            DirectoryEntry {
                name: "inner".to_owned(),
                is_dir: true,
                size: 0,
                modified: None,
                repository: Some(RepositoryInfo {
                    provider: Some("gitlab.com".to_owned()),
                    branch: Some("main".to_owned()),
                    remote: Some("git@gitlab.com:group/inner.git".to_owned()),
                    kind: RepositoryKind::Submodule {
                        outer: "/repos/outer".to_owned(),
                    },
                    last_activity: None,
                    last_fetch: None,
                }),
            },
        ];

        let mut buffer = Vec::new();
        write_message(&mut buffer, &Response::Directory { entries }).unwrap();
        let read: Response = read_message(&mut buffer.as_slice()).unwrap();

        let Response::Directory { entries } = read else {
            panic!("expected a listing");
        };
        assert_eq!(
            entries[0].repository.as_ref().unwrap().kind,
            RepositoryKind::Worktree {
                clone: "/repos/clone".to_owned(),
                clone_exists: true,
            }
        );
        assert_eq!(
            entries[1].repository.as_ref().unwrap().kind,
            RepositoryKind::Submodule {
                outer: "/repos/outer".to_owned(),
            }
        );
    }

    #[test]
    fn a_repository_from_before_kind_existed_reads_as_a_clone() {
        // `kind` is defaulted rather than required, so a front end built
        // after #587 can still read a `RepositoryInfo` from a service built
        // before it.
        let older = r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0,
            "modified":null,"repository":{"provider":null,"branch":"main","remote":null}}]}}"#;
        let response: Response = serde_json::from_str(older).unwrap();

        match response {
            Response::Directory { entries } => assert_eq!(
                entries[0].repository.as_ref().unwrap().kind,
                RepositoryKind::Clone
            ),
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn a_repository_from_before_last_activity_existed_reads_as_none() {
        // `last_activity` is defaulted rather than required, so a front end
        // built after #588 can still read a `RepositoryInfo` from a service
        // built before it.
        let older = r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0,
            "modified":null,"repository":{"provider":null,"branch":"main","remote":null}}]}}"#;
        let response: Response = serde_json::from_str(older).unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries[0].repository.as_ref().unwrap().last_activity, None);
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[test]
    fn a_repository_from_before_last_fetch_existed_reads_as_none() {
        // `last_fetch` is defaulted rather than required, so a front end
        // built after #589 can still read a `RepositoryInfo` from a service
        // built before it.
        let older = r#"{"Directory":{"entries":[{"name":"src","is_dir":true,"size":0,
            "modified":null,"repository":{"provider":null,"branch":"main","remote":null}}]}}"#;
        let response: Response = serde_json::from_str(older).unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries[0].repository.as_ref().unwrap().last_fetch, None);
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
            Request::FindCertificates,
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
            Response::Certificates {
                certificates: Vec::new(),
                complete: true,
            },
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
                    kind: RepositoryKind::Clone,
                    last_activity: None,
                    last_fetch: None,
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

    #[test]
    fn the_named_socket_differs_between_two_users() {
        assert_ne!(named_for_user("1000"), named_for_user("1001"));
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

    // ---- #680: what the two front ends say about the Repos Directory ----

    #[test]
    fn classify_root_problem_reports_a_missing_folder() {
        let missing = std::env::temp_dir().join("repos-explorer-680-missing-folder");
        let _ = std::fs::remove_dir_all(&missing);

        let problem = classify_root_problem(&missing, "not found");

        assert_eq!(
            problem,
            RootProblem::NotThere {
                cause: NotThereCause::FolderMissing
            }
        );
        assert!(problem.title(&missing).contains("is not available"));
        assert_eq!(problem.detail(), "The folder does not exist");
    }

    #[test]
    fn classify_root_problem_reports_a_drive_that_is_not_connected() {
        // The letter is found, never named. A named one - this project's
        // own "Z:" example - is a real mapped drive on some of the
        // machines that run these tests, where the classifier rightly
        // answers "unreadable" and the test fails, while a runner without
        // that drive stays green. Do not write a constant back.
        //
        // "Lacks" has to mean the letter's root answers `NotFound`, not
        // merely that it cannot be read: a drive that is present but not
        // ready, such as an empty optical drive, answers otherwise and is
        // classified unreadable, which is correct and is not the case
        // under test.
        let absent = ('A'..='Z').find(|letter| {
            let mut root = format!("{letter}:");
            root.push(std::path::MAIN_SEPARATOR);
            std::fs::metadata(root)
                .err()
                .is_some_and(|err| err.kind() == std::io::ErrorKind::NotFound)
        });
        let Some(absent) = absent else {
            // Every drive letter answers on this host, so there is no
            // missing drive to tell apart from a missing folder, and
            // nothing to assert.
            return;
        };

        let drive = format!("{absent}:");
        let mut root = drive.clone();
        root.push(std::path::MAIN_SEPARATOR);
        root.push_str("repos");

        let problem = classify_root_problem(Path::new(&root), "not found");

        assert_eq!(
            problem,
            RootProblem::NotThere {
                cause: NotThereCause::DriveNotConnected(drive.clone())
            }
        );
        assert_eq!(problem.detail(), format!("Drive {drive} is not connected"));
    }

    #[cfg(unix)]
    #[test]
    fn classify_root_problem_reports_permission_denied() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = std::env::temp_dir().join("repos-explorer-680-permission-denied");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = std::fs::read_dir(&dir).unwrap_err();

        let problem = classify_root_problem(&dir, &err.to_string());

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            problem,
            RootProblem::NotReadable {
                message: err.to_string(),
            }
        );
        assert!(problem.title(&dir).contains("cannot read"));
        assert_eq!(problem.detail(), err.to_string());
    }

    #[test]
    fn the_empty_root_title_names_the_path() {
        let root = Path::new("/home/ada/repos");

        assert!(empty_root_title(root).contains("has no repositories yet"));
        assert!(EMPTY_ROOT_DETAIL.contains("will appear here"));
    }
}
