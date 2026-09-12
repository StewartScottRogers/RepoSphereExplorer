//! The fat process: filesystem traversal, indexing, operations, and plugin cores.

pub mod repos;

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};
use plugin_api::{FolderCore, PluginCore};
use protocol::{DirectoryEntry, PluginView, Request, Response};
use serde::Serialize;
use std::fs;
use std::io;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

/// Number of bytes read from the start of a file when sniffing its type.
///
/// Large enough to cover ISO 9660's Primary Volume Descriptor, whose
/// `CD001` standard identifier sits at offset 32769 rather than near the
/// start of the file like every other sniffed format's magic.
const SNIFF_PREFIX_LEN: u64 = 32_774;

/// Every content-sniffed core plugin linked into this service, in sniffing
/// priority order. The directory plugin is dispatched separately (see
/// [`view_file`]) since it has no file bytes to sniff.
///
/// Hand-registered: a registration macro would be structure with no second
/// caller to justify it while twelve entries can still be read at a glance
/// (see `plugin-api`'s crate docs).
///
/// `rust` leads. Rust's own syntax satisfies markers several other plugins
/// sniff for - a `match` arm like `Some(x) => ...` reads as a JavaScript
/// arrow function, and a top-level `enum` or `type` alias reads as
/// TypeScript - so behind them it lost the great majority of real Rust
/// files to whichever sibling matched first. Its own markers (`fn `,
/// `impl `, `let mut `, `#[derive(`, `use std::`, `println!(`) are specific
/// enough to lead without taking anything from a sibling: `samples/`
/// pins that, since every fixture there must still be claimed by the
/// plugin whose directory it sits in.
const CORE_PLUGINS: &[&dyn PluginCore] = &[
    &plugin_rust::RustCore,
    &plugin_perl::PerlCore,
    &plugin_prolog::PrologCore,
    &plugin_php::PhpCore,
    &plugin_elixir::ElixirCore,
    &plugin_crystal::CrystalCore,
    &plugin_ruby::RubyCore,
    &plugin_python::PythonCore,
    &plugin_svelte::SvelteCore,
    &plugin_typescript::TypeScriptCore,
    &plugin_javascript::JavaScriptCore,
    &plugin_makefile::MakefileCore,
    &plugin_go::GoCore,
    &plugin_java::JavaCore,
    &plugin_kotlin::KotlinCore,
    &plugin_jenkinsfile::JenkinsfileCore,
    &plugin_groovy::GroovyCore,
    &plugin_csharp::CSharpCore,
    &plugin_vbnet::VbNetCore,
    &plugin_matlab::MatlabCore,
    &plugin_objective_c::ObjectiveCCore,
    &plugin_cpp::CppCore,
    &plugin_c::CCore,
    &plugin_swift::SwiftCore,
    &plugin_dockerfile::DockerfileCore,
    &plugin_shell::ShellCore,
    &plugin_powershell::PowerShellCore,
    &plugin_tcl::TclCore,
    &plugin_r::RCore,
    &plugin_haskell::HaskellCore,
    &plugin_fsharp::FSharpCore,
    &plugin_ocaml::OCamlCore,
    &plugin_nim::NimCore,
    &plugin_elm::ElmCore,
    &plugin_scala::ScalaCore,
    &plugin_sql::SqlCore,
    &plugin_clojure::ClojureCore,
    &plugin_scheme::SchemeCore,
    &plugin_dart::DartCore,
    &plugin_erlang::ErlangCore,
    &plugin_julia::JuliaCore,
    &plugin_fortran::FortranCore,
    &plugin_ada::AdaCore,
    &plugin_assembly::AssemblyCore,
    &plugin_vimscript::VimscriptCore,
    &plugin_graphql::GraphQlCore,
    &plugin_solidity::SolidityCore,
    &plugin_svg::SvgCore,
    &plugin_vue::VueCore,
    &plugin_html::HtmlCore,
    &plugin_maven::MavenCore,
    &plugin_msbuild::MsbuildCore,
    &plugin_xml::XmlCore,
    &plugin_restructuredtext::RestructuredTextCore,
    &plugin_jupyter_notebook::NotebookCore,
    &plugin_model3d::Model3dCore,
    &plugin_geojson::GeoJsonCore,
    &plugin_npmlock::NpmlockCore,
    &plugin_webmanifest::WebmanifestCore,
    &plugin_sourcemap::SourcemapCore,
    &plugin_jsonschema::JsonschemaCore,
    &plugin_openapi::OpenapiCore,
    &plugin_cloudformation::CloudformationCore,
    &plugin_jsonlines::JsonlinesCore,
    &plugin_json5::Json5Core,
    &plugin_json::JsonCore,
    &plugin_terraform::TerraformCore,
    &plugin_editorconfig::EditorconfigCore,
    &plugin_cargolock::CargolockCore,
    &plugin_toml::TomlCore,
    &plugin_csv::CsvCore,
    &plugin_msgpack::MsgpackCore,
    &plugin_certificate::CertificateCore,
    &plugin_markdown::MarkdownCore,
    &plugin_helmchart::HelmchartCore,
    &plugin_gitlabci::GitlabciCore,
    &plugin_githubactions::GithubactionsCore,
    &plugin_kubernetes::KubernetesCore,
    &plugin_yarnlock::YarnlockCore,
    &plugin_pnpmlock::PnpmlockCore,
    &plugin_ansible::AnsibleCore,
    &plugin_yaml::YamlCore,
    &plugin_gitconfig::GitconfigCore,
    &plugin_systemdunit::SystemdunitCore,
    // These three sit ahead of `ini`, which sniffs loosely enough to
    // claim all of them: an nginx, Apache or Caddy configuration is not
    // a narrower kind of INI file, so `specialises` would be a lie -
    // this is what the order in this list is for.
    &plugin_nginxconf::NginxconfCore,
    &plugin_apacheconf::ApacheconfCore,
    &plugin_caddyfile::CaddyfileCore,
    &plugin_sshconfig::SshconfigCore,
    &plugin_ini::IniCore,
    &plugin_properties::PropertiesCore,
    &plugin_diff::DiffCore,
    &plugin_asciidoc::AsciidocCore,
    &plugin_orgmode::OrgmodeCore,
    &plugin_latex::LatexCore,
    &plugin_bibtex::BibtexCore,
    &plugin_requirements::RequirementsCore,
    &plugin_dotenv::DotenvCore,
    &plugin_codeowners::CodeownersCore,
    &plugin_roff::RoffCore,
    &plugin_gitattributes::GitattributesCore,
    &plugin_ignorefile::IgnorefileCore,
    &plugin_protobuf::ProtobufCore,
    &plugin_thrift::ThriftCore,
    &plugin_flatbuffers::FlatbuffersCore,
    &plugin_antlr::AntlrCore,
    &plugin_yacc::YaccCore,
    &plugin_lex::LexCore,
    &plugin_solution::SolutionCore,
    &plugin_lua::LuaCore,
    &plugin_zig::ZigCore,
    &plugin_dlang::DlangCore,
    &plugin_pascal::PascalCore,
    &plugin_cobol::CobolCore,
    &plugin_verilog::VerilogCore,
    &plugin_vhdl::VhdlCore,
    &plugin_elisp::ElispCore,
    &plugin_awk::AwkCore,
    &plugin_batchfile::BatchfileCore,
    &plugin_purescript::PurescriptCore,
    &plugin_gleam::GleamCore,
    &plugin_text::TextCore,
    &plugin_image::ImageCore,
    &plugin_psd::PsdCore,
    &plugin_font::FontCore,
    &plugin_executable::ExecutableCore,
    &plugin_wasm::WasmCore,
    &plugin_word_document::WordDocumentCore,
    &plugin_spreadsheet::SpreadsheetCore,
    &plugin_presentation::PresentationCore,
    &plugin_epub::EpubCore,
    &plugin_comic_archive::ComicArchiveCore,
    &plugin_video::VideoCore,
    &plugin_audio::AudioCore,
    &plugin_archive::ArchiveCore,
    &plugin_pdf::PdfCore,
    &plugin_parquet::ParquetCore,
    &plugin_avro::AvroCore,
    &plugin_sqlite::SqliteCore,
    &plugin_hdf5::Hdf5Core,
    &plugin_disk_image::DiskImageCore,
    &plugin_package_archive::PackageArchiveCore,
];

/// The directory-as-file plugin, dispatched directly by [`view_file`] when
/// the path is a directory rather than through content sniffing.
const DIRECTORY_PLUGIN: &dyn PluginCore = &plugin_directory::DirectoryCore;

/// The folder plugins, which say what kind of programming project a folder
/// holds.
///
/// Every one that recognises a folder contributes, which is the difference
/// between these and [`CORE_PLUGINS`]. A file has one type, and two
/// plugins claiming one file is a defect. A folder is several things at
/// once: this repository's own root is a source control working copy and a
/// Cargo workspace, and neither description is the wrong one.
const FOLDER_PLUGINS: &[&dyn FolderCore] = &[&plugin_project_cargo::CargoProjectCore];

/// Every folder plugin that recognises the folder at `path`, in
/// registration order.
///
/// One directory read, for the selected folder only. A listing does not
/// call this: a `.git` check is a single `metadata` call per row, but this
/// is a full directory read per row, and a Repos Directory holding two
/// hundred folders would pay it two hundred times before a row drew. See
/// GUIDANCE.md section 2.5.
fn folder_plugins_for(path: &Path) -> Vec<&'static dyn FolderCore> {
    let Ok(entries) = fs::read_dir(path) else {
        return Vec::new();
    };
    let names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();

    folder_plugins_among(FOLDER_PLUGINS, &names)
}

/// Every plugin in `plugins` that recognises a folder holding `names`.
///
/// Split from the directory read so the rule it carries - *all* of them,
/// not the first - is testable against more than one matching plugin.
/// There is one folder plugin registered today, so a `find` here would
/// pass every test that only used the real registry, and would then
/// silently drop the second description the moment a second plugin
/// existed.
fn folder_plugins_among(
    plugins: &[&'static dyn FolderCore],
    names: &[&str],
) -> Vec<&'static dyn FolderCore> {
    plugins
        .iter()
        .filter(|plugin| plugin.sniff(names))
        .copied()
        .collect()
}

/// Lists the immediate contents of `path`, sorted by name without regard
/// to case, so a capitalised entry sits among its neighbours rather than
/// ahead of every lowercase one. Names differing only in case keep a
/// stable order between them.
///
/// # Errors
/// Returns an error if `path` cannot be read as a directory.
pub fn list_directory(path: &Path) -> io::Result<Vec<DirectoryEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_dir = entry.file_type()?.is_dir();
        let metadata = entry.metadata()?;
        let size = metadata.len();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs());
        // What a directory *is* to this application: a working copy, or an
        // ordinary folder that stays listed either way (GUIDANCE.md 2.5).
        let repository = if is_dir {
            repos::describe(&entry.path())
        } else {
            None
        };
        entries.push(DirectoryEntry {
            name,
            is_dir,
            size,
            modified,
            repository,
        });
    }
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

/// Reads a bounded prefix from the start of the file at `path`.
fn read_prefix(path: &Path) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(SNIFF_PREFIX_LEN).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Runs one call into a plugin behind a boundary that catches an unwind.
///
/// Eighty plugin cores are linked into this process, and behind them sit
/// `psd`, `lopdf`, `matroska`, `mp4`, `lofty`, `parquet`, `hdf5`,
/// `ttf-parser` and the rest - every one of them parsing untrusted bytes.
/// A bounds slip in any of them used to unwind straight out of the request
/// handler and end the service, which is the one process that can read the
/// filesystem: both front ends lose everything, over one unreadable file.
/// This is a boundary, not a recovery. The preview is lost either way; the
/// service is not.
///
/// `AssertUnwindSafe` is the honest choice here: the plugin owns no state
/// this process keeps, so there is nothing for a half-finished call to
/// leave inconsistent.
fn guarded<T>(plugin: &str, path: &Path, call: impl FnOnce() -> T) -> Result<T, io::Error> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|payload| {
        let detail = panic_detail(&payload);
        let message = format!(
            "the {plugin} plugin panicked reading {}: {detail}",
            path.display()
        );
        journal(
            "plugin-panic",
            &[path.display().to_string()],
            &Err(io::Error::other(message.clone())),
        );
        io::Error::new(io::ErrorKind::InvalidData, message)
    })
}

/// What a caught panic said, when it said anything a string can hold.
fn panic_detail(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned())
}

/// Finds the first registered plugin that recognises `path`'s content.
fn sniff(path: &Path) -> io::Result<Option<&'static dyn PluginCore>> {
    let prefix = read_prefix(path)?;
    Ok(sniff_among(CORE_PLUGINS, path, &prefix))
}

/// Which of `plugins` should view `path`, given its `prefix`.
///
/// Split out from [`sniff`] so a test can offer a file to a list of its
/// own - including a plugin that panics, which is the case worth pinning
/// and the one a static list cannot express.
fn sniff_among<'a>(
    plugins: &[&'a dyn PluginCore],
    path: &Path,
    prefix: &[u8],
) -> Option<&'a dyn PluginCore> {
    // A plugin that panics while sniffing declines the file: the ones after
    // it in the list still get their turn, which is the whole point of
    // asking all of them.
    let matches: Vec<&'a dyn PluginCore> = plugins
        .iter()
        .filter(|plugin| guarded(plugin.name(), path, || plugin.sniff(prefix)).unwrap_or(false))
        .copied()
        .collect();
    let matches = most_specific(&matches);
    claimed_by_extension(path, &matches).or_else(|| matches.first().copied())
}

/// The matches that refine another match, or all of them when none does.
///
/// An npm lock file is JSON, so both plugins recognise it - and `json`
/// owns the extension, which [`claimed_by_extension`] would otherwise
/// settle it on.
///
/// This *keeps* the specialisations rather than merely dropping what they
/// refine, and the difference is not academic. Removing the general plugin
/// alone promoted whatever unrelated plugin happened to match earliest: a
/// Kubernetes manifest went to `sql`, which reads a `---` document marker
/// as a comment, because dropping `yaml` left `sql` first in the list. A
/// plugin that recognised the file *and* says it is a narrower reading of
/// another plugin that also recognised it is strictly the better answer,
/// so that is what survives.
fn most_specific<'a>(matches: &[&'a dyn PluginCore]) -> Vec<&'a dyn PluginCore> {
    let names: Vec<&'static str> = matches.iter().map(|plugin| plugin.name()).collect();
    let specialisations: Vec<&'a dyn PluginCore> = matches
        .iter()
        .filter(|plugin| {
            plugin
                .specialises()
                .iter()
                .any(|refined| names.contains(refined))
        })
        .copied()
        .collect();
    if specialisations.is_empty() {
        matches.to_vec()
    } else {
        specialisations
    }
}

/// Whichever of `matches` claims `path`'s extension, if one does.
///
/// GUIDANCE.md §3.3 makes the extension "a hint only", and this is the whole
/// of that hint: it chooses between plugins that all recognised the content,
/// and never overrules them. A file whose extension nobody claims, or whose
/// claimant did not recognise the content, is left to priority order - so a
/// PNG named `.txt` is still a PNG.
///
/// Ties are real rather than hypothetical among the source languages, which
/// have no magic bytes to sniff: `struct` is C, C++, Rust, Swift and
/// Solidity; `package` is Java, Go and Perl; `class` is a dozen of them.
fn claimed_by_extension<'a>(
    path: &Path,
    matches: &[&'a dyn PluginCore],
) -> Option<&'a dyn PluginCore> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    matches
        .iter()
        .find(|plugin| plugin.extensions().contains(&extension.as_str()))
        .copied()
}

/// Views the path through whichever registered plugin recognises it: the
/// directory plugin if `path` is a directory, otherwise whichever content
/// plugin's `sniff` matches.
///
/// # Errors
/// Returns an error if `path` cannot be read.
pub fn view_file(path: &Path) -> io::Result<Response> {
    if fs::metadata(path)?.is_dir() {
        let name = DIRECTORY_PLUGIN.name();
        let mut also = Vec::new();
        for plugin in folder_plugins_for(path) {
            let project = plugin.name();
            // A folder plugin that panics or cannot read its manifest
            // costs the folder that one description and nothing else: the
            // folder is still a folder, and the rest of the pane stands.
            if let Ok(Ok(data)) = guarded(project, path, || plugin.view(path)) {
                also.push(PluginView {
                    plugin: project.to_owned(),
                    data,
                });
            }
        }
        return Ok(Response::FileView {
            plugin: name.to_owned(),
            data: guarded(name, path, || DIRECTORY_PLUGIN.view(path))??,
            also,
        });
    }
    Ok(match sniff(path)? {
        Some(plugin) => {
            let name = plugin.name();
            match guarded(name, path, || plugin.view(path)) {
                Ok(data) => Response::FileView {
                    plugin: name.to_owned(),
                    data: data?,
                    also: Vec::new(),
                },
                // A panicking plugin costs this file its preview and
                // nothing else: the caller can go straight on to the next.
                Err(err) => Response::Error {
                    message: err.to_string(),
                },
            }
        }
        None => Response::Error {
            message: format!("no plugin recognises {}", path.display()),
        },
    })
}

/// Lists `path` if it is a directory, otherwise views it through whichever
/// registered plugin recognises it.
fn open(path: &Path) -> Response {
    let result = match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {
            list_directory(path).map(|entries| Response::Directory { entries })
        }
        Ok(_) => view_file(path),
        Err(err) => Err(err),
    };
    result.unwrap_or_else(|err| Response::Error {
        message: err.to_string(),
    })
}

/// Where operations are journaled by default:
/// `<data-local-dir>/RepoSphereExplorer/journal.jsonl`.
fn default_journal_path() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("RepoSphereExplorer").join("journal.jsonl"))
}

/// One line of the operations journal: GUIDANCE.md §2.1.5 requires
/// destructive operations to be "journaled so the action can be described
/// after the fact".
#[derive(Debug, Serialize)]
struct JournalEntry<'a> {
    at_unix_secs: u64,
    operation: &'a str,
    targets: &'a [String],
    outcome: &'a str,
}

/// Appends one line describing `operation` on `targets` to the journal at
/// `path`. Best-effort: a journaling failure is reported to stderr and
/// never propagated, so it can't block the operation it's recording.
fn journal_to(path: &Path, operation: &str, targets: &[String], outcome: &io::Result<()>) {
    let entry = JournalEntry {
        at_unix_secs: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs()),
        operation,
        targets,
        outcome: &match outcome {
            Ok(()) => "ok".to_owned(),
            Err(err) => err.to_string(),
        },
    };
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match fs::OpenOptions::new().create(true).append(true).open(path) {
        Ok(mut file) => {
            if let Err(err) = writeln!(file, "{line}") {
                eprintln!("could not write journal entry: {err}");
            }
        }
        Err(err) => eprintln!("could not open journal at {}: {err}", path.display()),
    }
}

/// Appends one line to the default journal, if one is resolvable. See
/// [`journal_to`].
fn journal(operation: &str, targets: &[String], outcome: &io::Result<()>) {
    if let Some(path) = default_journal_path() {
        journal_to(&path, operation, targets, outcome);
    }
}

/// Fails if anything already exists at `path`, so an operation that would
/// silently replace it stops before touching the filesystem.
fn refuse_if_exists(path: &Path) -> io::Result<()> {
    if path.symlink_metadata().is_ok() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists", path.display()),
        ));
    }
    Ok(())
}

/// As [`refuse_if_exists`], but lets `to` be `from` itself: both front ends
/// pre-fill the rename prompt with the current name, so renaming a path to
/// the name it already has is the no-op a user gets by pressing Enter
/// without editing, not an attempt to replace anything.
fn refuse_if_occupied_by_another(from: &Path, to: &Path) -> io::Result<()> {
    match (from.canonicalize(), to.canonicalize()) {
        (Ok(from), Ok(to)) if from == to => Ok(()),
        _ => refuse_if_exists(to),
    }
}

/// What would undo the last operation, if it can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Undoable {
    /// Move `to` back to `from`, undoing a rename or a move.
    Rename { from: PathBuf, to: PathBuf },
    /// Remove what an operation created: a copy's destination, a new file
    /// or folder, an extracted directory.
    Remove { path: PathBuf },
    /// Put a file's previous text back, undoing an edit.
    Restore { path: PathBuf, content: String },
}

thread_local! {
    /// The single step [`Request::Undo`] would take. GUIDANCE.md §2 keeps
    /// business rules out of the front ends, so the service remembers what
    /// the last operation was and how to reverse it; a front end only asks.
    ///
    /// One step deep, per D6's "undo of the immediately preceding
    /// operation". Thread-local because the service answers every request on
    /// one thread ([`run`] loops over [`serve_one`]), which also keeps the
    /// tests from treading on each other's step.
    static UNDO: std::cell::RefCell<Option<Undoable>> = const { std::cell::RefCell::new(None) };
}

/// Records `step` as what would undo the operation just performed, or
/// clears the record when the operation cannot be undone.
fn remember_undo(step: Option<Undoable>) {
    UNDO.with_borrow_mut(|slot| *slot = step);
}

/// Records an undo step only if `result` succeeded; a failed operation
/// changed nothing and leaves the previous step alone.
fn remember_if_done(result: &io::Result<()>, step: Undoable) {
    if result.is_ok() {
        remember_undo(Some(step));
    }
}

/// Undoes the last operation, journaling the attempt.
///
/// # Errors
/// Returns an error if there is nothing to undo, or if reversing it fails.
pub fn undo() -> io::Result<()> {
    let step = UNDO.with_borrow_mut(Option::take);
    let Some(step) = step else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "nothing to undo. A delete is undone from the Recycle Bin",
        ));
    };
    let (operation, targets, result) = match step {
        Undoable::Rename { from, to } => {
            let targets = vec![from.display().to_string(), to.display().to_string()];
            let result = refuse_if_occupied_by_another(&from, &to)
                .and_then(|()| std::fs::rename(&from, &to));
            ("undo_rename", targets, result)
        }
        Undoable::Remove { path } => {
            let targets = vec![path.display().to_string()];
            // To the recycle bin, not erased: undoing a copy should be no
            // more destructive than the copy was.
            let result = trash::delete(&path).map_err(|err| io::Error::other(err.to_string()));
            ("undo_create", targets, result)
        }
        Undoable::Restore { path, content } => {
            let targets = vec![path.display().to_string()];
            let result = write_atomically(&path, &content);
            ("undo_edit", targets, result)
        }
    };
    journal(operation, &targets, &result);
    result
}

/// Renames (moves) `from` to `to`, journaling the attempt. Refuses to
/// replace an existing `to`, the way [`create_file`] and
/// [`create_directory`] refuse an existing target: the front ends drive
/// this from a free-text prompt, where a name that happens to match a
/// sibling would otherwise destroy it without a word. Renaming a path to
/// the name it already has stays the no-op it was.
///
/// # Errors
/// Returns an error if something other than `from` already exists at `to`,
/// or if the rename fails.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let result = refuse_if_occupied_by_another(from, to).and_then(|()| fs::rename(from, to));
    remember_if_done(
        &result,
        Undoable::Rename {
            from: to.to_path_buf(),
            to: from.to_path_buf(),
        },
    );
    journal(
        "rename",
        &[from.display().to_string(), to.display().to_string()],
        &result,
    );
    result
}

/// Copies the file at `from` to `to`, journaling the attempt. Refuses to
/// replace an existing `to`, for the same reason [`rename`] does.
///
/// # Errors
/// Returns an error if something already exists at `to`, or if the copy
/// fails.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    let result = refuse_if_exists(to).and_then(|()| fs::copy(from, to).map(|_| ()));
    remember_if_done(
        &result,
        Undoable::Remove {
            path: to.to_path_buf(),
        },
    );
    journal(
        "copy",
        &[from.display().to_string(), to.display().to_string()],
        &result,
    );
    result
}

/// Writes `content` to `path` by writing a temporary file beside it and
/// renaming it into place, so an interrupted write leaves the original
/// intact rather than a half-written file.
fn write_atomically(path: &Path, content: &str) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".rse-write");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, content)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            // Leave nothing behind if the rename is the part that failed.
            let _ = fs::remove_file(&temporary);
            Err(err)
        }
    }
}

/// Replaces the text of the existing file at `path`, journaling the attempt.
/// Refuses a path that is not already a file: an editor saves over something
/// it opened, and creating one is `create_file`'s job.
///
/// # Errors
/// Returns an error if `path` is not an existing file, or the write fails.
pub fn write_file(path: &Path, content: &str) -> io::Result<()> {
    let result = (|| {
        if !fs::metadata(path)?.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a file", path.display()),
            ));
        }
        // Read the old text first: without it the edit cannot be undone, and
        // an edit that cannot be undone is the one operation here that
        // destroys work rather than moving it.
        let previous = fs::read_to_string(path)?;
        write_atomically(path, content)?;
        remember_undo(Some(Undoable::Restore {
            path: path.to_path_buf(),
            content: previous,
        }));
        Ok(())
    })();
    journal("write_file", &[path.display().to_string()], &result);
    result
}

/// Deletes every path in `paths` - the exact, confirmed target set per
/// GUIDANCE.md §2.1.5, never a pattern the service resolves itself -
/// journaling the attempt. Targets are moved to the OS trash/recycle bin
/// per GUIDANCE.md §2.3, not permanently erased.
///
/// # Errors
/// Returns an error if any path cannot be moved to the trash; earlier
/// paths in the list may already have been moved.
pub fn delete(paths: &[String]) -> io::Result<()> {
    let result = (|| {
        for path in paths {
            trash::delete(path).map_err(|err| io::Error::other(err.to_string()))?;
        }
        Ok(())
    })();
    // A delete goes to the recycle bin, which this service cannot pull back
    // out; leaving a stale step here would undo the wrong thing.
    remember_undo(None);
    journal("delete", paths, &result);
    result
}

/// Extracts the archive at `archive` into `destination`, journaling the
/// attempt.
///
/// # Errors
/// Returns an error if the archive cannot be extracted.
pub fn extract(archive: &Path, destination: &Path) -> io::Result<()> {
    let result = plugin_archive::extract(archive, destination);
    remember_if_done(
        &result,
        Undoable::Remove {
            path: destination.to_path_buf(),
        },
    );
    journal(
        "extract",
        &[
            archive.display().to_string(),
            destination.display().to_string(),
        ],
        &result,
    );
    result
}

/// Creates exactly one new, empty directory at `path`, journaling the
/// attempt. Does not create missing parent directories.
///
/// # Errors
/// Returns an error if the directory cannot be created, e.g. because
/// something already exists at `path` or its parent doesn't exist.
pub fn create_directory(path: &Path) -> io::Result<()> {
    let result = fs::create_dir(path);
    remember_if_done(
        &result,
        Undoable::Remove {
            path: path.to_path_buf(),
        },
    );
    journal("create_directory", &[path.display().to_string()], &result);
    result
}

/// Creates a new, empty file at `path`, journaling the attempt. Fails
/// rather than truncating if something already exists at `path`.
///
/// # Errors
/// Returns an error if the file cannot be created, e.g. because something
/// already exists at `path` or its parent doesn't exist.
pub fn create_file(path: &Path) -> io::Result<()> {
    let result = fs::File::create_new(path).map(|_| ());
    remember_if_done(
        &result,
        Undoable::Remove {
            path: path.to_path_buf(),
        },
    );
    journal("create_file", &[path.display().to_string()], &result);
    result
}

/// Runs `operation` and turns its result into a [`Response`].
fn respond_to_operation(operation: io::Result<()>) -> Response {
    match operation {
        Ok(()) => Response::Done,
        Err(err) => Response::Error {
            message: err.to_string(),
        },
    }
}

/// Computes the response for one request.
#[must_use]
pub fn handle_request(request: &Request) -> Response {
    match request {
        Request::ListDirectory { path } => match list_directory(Path::new(path)) {
            Ok(entries) => Response::Directory { entries },
            Err(err) => Response::Error {
                message: err.to_string(),
            },
        },
        Request::ViewFile { path } => {
            view_file(Path::new(path)).unwrap_or_else(|err| Response::Error {
                message: err.to_string(),
            })
        }
        Request::Open { path } => open(Path::new(path)),
        Request::Rename { from, to } => {
            respond_to_operation(rename(Path::new(from), Path::new(to)))
        }
        Request::Copy { from, to } => respond_to_operation(copy(Path::new(from), Path::new(to))),
        Request::Delete { paths } => respond_to_operation(delete(paths)),
        Request::Extract {
            archive,
            destination,
        } => respond_to_operation(extract(Path::new(archive), Path::new(destination))),
        Request::CreateDirectory { path } => {
            respond_to_operation(create_directory(Path::new(path)))
        }
        Request::CreateFile { path } => respond_to_operation(create_file(Path::new(path))),
        Request::WriteFile { path, content } => {
            respond_to_operation(write_file(Path::new(path), content))
        }
        Request::Undo => respond_to_operation(undo()),
        Request::ReposRoots => Response::ReposRoots {
            roots: repos::roots(),
            default: repos::default_root().to_string_lossy().into_owned(),
        },
        Request::SetReposRoot { path } => {
            let target = Path::new(path);
            let outcome = repos::set_active_root(target);
            journal("set-repos-root", &[target.display().to_string()], &outcome);
            respond_to_operation(outcome)
        }
    }
}

/// Starts listening on the local socket identified by `name`.
///
/// # Errors
/// Returns an error if the socket is already in use or cannot be created.
pub fn bind(name: Name<'_>) -> io::Result<Listener> {
    ListenerOptions::new().name(name).create_sync()
}

/// Accepts one connection on `listener`, answers exactly one request on it,
/// then returns.
///
/// # Errors
/// Returns an error if accepting the connection or the request/response
/// round trip fails.
pub fn serve_one(listener: &Listener) -> io::Result<()> {
    let mut conn: Stream = listener.accept()?;
    let request: Request = protocol::read_message(&mut conn)?;
    let response = handle_request(&request);
    protocol::write_message(&mut conn, &response)
}

/// Runs the service loop: accepts connections and answers one request on
/// each, forever.
///
/// # Errors
/// Never returns `Ok`; this signature only exists so callers can use `?`.
pub fn run(listener: &Listener) -> io::Result<()> {
    loop {
        if let Err(err) = serve_one(listener) {
            eprintln!("connection error: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CORE_PLUGINS, FolderCore, Path, bind, copy, create_directory, create_file, delete, extract,
        folder_plugins_among, guarded, handle_request, journal_to, list_directory, most_specific,
        open, rename, serve_one, sniff_among, undo, view_file, write_file,
    };
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
    use plugin_api::PluginCore;
    use protocol::{Request, Response};
    use std::fs;
    use std::io;

    fn unique_socket_name() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "rse-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    /// Writes `content` to a uniquely named file ending in `name`, and
    /// returns which plugin `view_file` attributes it to.
    fn plugin_for(name: &str, content: &[u8]) -> String {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        let plugin = match view_file(&path).unwrap() {
            Response::FileView { plugin, .. } => plugin,
            other => panic!("expected a file view, got {other:?}"),
        };
        fs::remove_dir_all(&dir).unwrap();
        plugin
    }

    #[test]
    fn a_source_file_is_attributed_to_its_own_language() {
        // Each of these went to the wrong plugin when the first content
        // match won outright: C and C++ to `rust` on their `struct` lines,
        // C# to `javascript`, Java to `perl` on its `package` line.
        let cases: [(&str, &[u8], &str); 4] = [
            (
                "ring.c",
                b"#include <stdio.h>

struct ring_slot {
    unsigned long sequence;
};

int main(void) {
    return 0;
}
",
                "c",
            ),
            (
                "matrix.cpp",
                b"#include <iostream>

struct Identity {
    static int one() { return 1; }
};

int main() {
    std::cout << Identity::one();
}
",
                "cpp",
            ),
            (
                "Inventory.cs",
                b"using System;

namespace Warehouse
{
    public class Inventory
    {
        public int Count { get; set; }
    }
}
",
                "csharp",
            ),
            (
                "OrderBook.java",
                b"package com.example.trading;

import java.util.List;

public class OrderBook {
    public static void main(String[] args) {
        System.out.println(\"hello\");
    }
}
",
                "java",
            ),
        ];

        for (name, content, expected) in cases {
            assert_eq!(
                plugin_for(name, content),
                expected,
                "{name} should open in its own language's plugin"
            );
        }
    }

    #[test]
    fn content_still_decides_when_the_extension_disagrees() {
        // A PNG called `.txt`: the extension's owner never recognised the
        // content, so the hint has nothing to choose between and the magic
        // bytes win, exactly as before. A whole 1x1 PNG, not just a header,
        // since attribution is only half the job - the plugin then reads it.
        let png = [
            0x89u8, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0x38, 0x21, 0x27, 0x07, 0x00, 0x02, 0xb6, 0x01, 0x05, 0x0a, 0x5b, 0xa6,
            0x06, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        assert_eq!(plugin_for("secretly-a-picture.txt", &png), "image");
    }

    #[test]
    fn an_extension_nobody_claims_is_attributed_by_content_alone() {
        assert_eq!(
            plugin_for(
                "notes.unheard-of",
                b"fn main() {
    let mut total = 0;
    println!(\"{total}\");
}
"
            ),
            "rust",
            "an unclaimed extension leaves priority order exactly as it was"
        );
    }

    #[test]
    fn no_two_plugins_claim_the_same_extension() {
        let mut seen: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
        for plugin in CORE_PLUGINS {
            for extension in plugin.extensions() {
                if let Some(other) = seen.insert(extension, plugin.name()) {
                    panic!(
                        "`{extension}` is claimed by both {other} and {}, so the hint cannot break a tie",
                        plugin.name()
                    );
                }
            }
        }
    }

    #[test]
    fn every_claimed_extension_is_lowercase_and_undotted() {
        for plugin in CORE_PLUGINS {
            for extension in plugin.extensions() {
                assert!(
                    !extension.is_empty()
                        && !extension.starts_with('.')
                        && extension.chars().all(|c| !c.is_ascii_uppercase()),
                    "{}: `{extension}` will never match a path's extension",
                    plugin.name()
                );
            }
        }
    }

    /// A plugin that panics the moment it is asked to read anything - the
    /// shape of a bounds slip inside a third-party parser, of which this
    /// process links a great many.
    #[derive(Debug)]
    struct PanickingCore;

    impl PluginCore for PanickingCore {
        fn name(&self) -> &'static str {
            "panicking"
        }

        fn sniff(&self, _prefix: &[u8]) -> bool {
            panic!("sniff went out of bounds");
        }

        fn view(&self, _path: &std::path::Path) -> io::Result<serde_json::Value> {
            panic!("view went out of bounds");
        }
    }

    /// A plugin that claims everything and reads nothing, to stand behind
    /// the panicking one in a list.
    #[derive(Debug)]
    struct AlwaysCore;

    impl PluginCore for AlwaysCore {
        fn name(&self) -> &'static str {
            "always"
        }

        fn sniff(&self, _prefix: &[u8]) -> bool {
            true
        }

        fn view(&self, _path: &std::path::Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "read": true }))
        }
    }

    #[test]
    fn a_panicking_plugin_yields_an_error_naming_the_file_and_the_plugin() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("trips-a-parser.bin");
        fs::write(&path, b"anything").unwrap();

        let outcome = guarded("panicking", &path, || PanickingCore.view(&path));

        let err = outcome.expect_err("the panic should have been caught");
        let message = err.to_string();
        assert!(message.contains("panicking"), "{message}");
        assert!(message.contains("trips-a-parser.bin"), "{message}");
        assert!(
            message.contains("sniff went out of bounds")
                || message.contains("view went out of bounds"),
            "{message}"
        );

        // And the process is still here to say so.
        assert!(view_file(&path).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_plugin_that_panics_while_sniffing_does_not_stop_the_others() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.bin");
        fs::write(&path, b"anything").unwrap();

        let plugins: [&dyn PluginCore; 2] = [&PanickingCore, &AlwaysCore];
        let chosen = sniff_among(&plugins, &path, b"anything");

        assert_eq!(
            chosen.map(PluginCore::name),
            Some("always"),
            "the plugin after the panicking one must still get its turn"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_service_keeps_working_after_a_plugin_panics() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("after.txt");
        fs::write(
            &path,
            b"still readable
",
        )
        .unwrap();

        // A panic caught, then an ordinary file viewed through the real
        // plugin list: the second call is the one that matters.
        let _ = guarded("panicking", &path, || PanickingCore.view(&path));

        match view_file(&path).unwrap() {
            Response::FileView { plugin, .. } => assert_eq!(plugin, "text"),
            other => panic!("expected a file view, got {other:?}"),
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_ordinary_plugin_error_is_left_alone() {
        // The boundary catches unwinds, not `Err`: a plugin that reports a
        // problem the normal way still reports it the normal way.
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nothing.txt");

        let outcome: io::Result<io::Result<serde_json::Value>> =
            guarded("text", &path, || Err(io::Error::other("no such thing")));

        let inner = outcome.expect("no panic, so no boundary error");
        assert_eq!(inner.unwrap_err().to_string(), "no such thing");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_registered_plugin_is_asked_behind_the_boundary() {
        // Not a behaviour test: a reminder that the list is what is exposed
        // to untrusted bytes, and that its size is the reason the boundary
        // exists.
        assert!(
            CORE_PLUGINS.len() > 50,
            "eighty parsers behind one process is the exposure being bounded"
        );
    }

    #[test]
    fn lists_a_directory_sorted_by_name() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("b.txt"), b"").unwrap();
        fs::write(dir.join("a.txt"), b"").unwrap();

        let entries = list_directory(&dir).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["a.txt", "b.txt", "sub"]
        );
        assert!(entries.iter().find(|e| e.name == "sub").unwrap().is_dir);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sorts_a_listing_without_regard_to_case() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(dir.join("New folder")).unwrap();
        fs::write(dir.join("alpha.txt"), b"").unwrap();
        fs::write(dir.join("Zebra.txt"), b"").unwrap();

        let entries = list_directory(&dir).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["alpha.txt", "New folder", "Zebra.txt"]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_a_file_s_size_and_modified_time() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file.txt"), b"hello world").unwrap();

        let entries = list_directory(&dir).unwrap();

        let entry = entries.iter().find(|e| e.name == "file.txt").unwrap();
        assert_eq!(entry.size, "hello world".len() as u64);
        assert!(entry.modified.is_some());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn views_a_text_file_through_the_text_plugin() {
        let path = std::env::temp_dir().join(unique_socket_name());
        fs::write(&path, "hello\nworld\n").unwrap();

        let response = view_file(&path).unwrap();

        match response {
            Response::FileView { plugin, data, .. } => {
                assert_eq!(plugin, "text");
                assert_eq!(data["content"], "hello\nworld\n");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn views_a_directory_through_the_directory_plugin() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("note.txt"), b"hi").unwrap();

        let response = view_file(&dir).unwrap();

        match response {
            Response::FileView { plugin, data, .. } => {
                assert_eq!(plugin, "directory");
                assert_eq!(data["entry_count"], 1);
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opens_a_directory_as_a_listing_and_a_file_as_a_view() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        fs::write(&file, "hi").unwrap();

        assert!(matches!(open(&dir), Response::Directory { .. }));
        assert!(matches!(open(&file), Response::FileView { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rust_leads_the_plugins_whose_markers_its_own_syntax_matches() {
        // A `match` arm on a tuple pattern reads as a JavaScript arrow
        // function, and a top-level `enum` or `type` alias reads as
        // TypeScript. Both plugins used to sit ahead of `rust`.
        let source = concat!(
            "type Name = String;
",
            "enum Volume {
",
            "    Quiet,
",
            "}
",
            "fn parse(text: &str) -> Name {
",
            "    match text.split_once(',') {
",
            "        Some((name, _rest)) => name.to_owned(),
",
            "        None => text.to_owned(),
",
            "    }
",
            "}
",
        );

        let plugin = super::CORE_PLUGINS
            .iter()
            .find(|plugin| plugin.sniff(source.as_bytes()))
            .expect("some plugin should recognise Rust source");

        assert_eq!(plugin.name(), "rust");
    }

    #[test]
    fn renames_a_real_file() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("old.txt");
        let to = dir.join("new.txt");
        fs::write(&from, "content").unwrap();

        rename(&from, &to).unwrap();

        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copies_a_real_file_leaving_the_source_in_place() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("copy.txt");
        fs::write(&from, "content").unwrap();

        copy(&from, &to).unwrap();

        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_to_rename_onto_an_existing_path() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("old.txt");
        let to = dir.join("occupied.txt");
        fs::write(&from, "content").unwrap();
        fs::write(&to, "precious").unwrap();

        let err = rename(&from, &to).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert_eq!(fs::read_to_string(&to).unwrap(), "precious");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_to_copy_onto_an_existing_path() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("occupied.txt");
        fs::write(&from, "content").unwrap();
        fs::write(&to, "precious").unwrap();

        let err = copy(&from, &to).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&to).unwrap(), "precious");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_path_to_its_own_name_is_a_no_op() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unchanged.txt");
        fs::write(&path, "content").unwrap();

        rename(&path, &path).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writing_replaces_a_file_s_text_and_the_edit_can_be_undone() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.txt");
        fs::write(&path, "before").unwrap();

        write_file(&path, "after").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "after");

        undo().unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "before",
            "an edit is the one operation that destroys work, so it undoes"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writing_refuses_a_path_that_is_not_an_existing_file() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();

        // A directory, and a file that is not there at all.
        assert!(write_file(&dir, "text").is_err());
        assert!(write_file(&dir.join("absent.txt"), "text").is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_no_temporary_file_beside_the_original() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("absent.txt");

        let _ = write_file(&absent, "text");

        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(leftovers.is_empty(), "nothing left behind: {leftovers:?}");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_puts_a_renamed_file_back() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("before.txt");
        let to = dir.join("after.txt");
        fs::write(&from, "content").unwrap();
        rename(&from, &to).unwrap();

        undo().unwrap();

        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert!(!to.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_removes_what_a_copy_created_and_leaves_the_source() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("copy.txt");
        fs::write(&from, "content").unwrap();
        copy(&from, &to).unwrap();

        undo().unwrap();

        assert!(!to.exists(), "the copy is gone");
        assert_eq!(
            fs::read_to_string(&from).unwrap(),
            "content",
            "the original is untouched"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_removes_a_newly_created_folder() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let created = dir.join("New folder");
        create_directory(&created).unwrap();

        undo().unwrap();

        assert!(!created.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_goes_only_one_step_and_says_so_when_there_is_nothing_left() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        create_directory(&dir.join("one")).unwrap();

        undo().unwrap();
        let err = undo().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("Recycle Bin"),
            "and points at where a delete goes instead: {err}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_delete_leaves_nothing_to_undo() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let doomed = dir.join("doomed.txt");
        fs::write(&doomed, "content").unwrap();
        // A create would otherwise be the step on record.
        create_directory(&dir.join("earlier")).unwrap();

        delete(&[doomed.to_string_lossy().into_owned()]).unwrap();

        let err = undo().unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::NotFound,
            "the earlier create must not be undone in a delete's place"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_operation_leaves_the_previous_undo_step_alone() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let created = dir.join("kept");
        create_directory(&created).unwrap();
        // Refused, because something already exists there.
        create_directory(&created).unwrap_err();

        undo().unwrap();

        assert!(!created.exists(), "the successful create is still undoable");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_a_new_directory() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let new_dir = dir.join("sub");

        create_directory(&new_dir).unwrap();

        assert!(new_dir.is_dir());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creating_a_directory_over_an_existing_path_errors_via_handle_request() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("sub");
        fs::create_dir_all(&existing).unwrap();

        let request = Request::CreateDirectory {
            path: existing.to_string_lossy().into_owned(),
        };

        assert!(matches!(handle_request(&request), Response::Error { .. }));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creates_a_new_file() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let new_file = dir.join("note.txt");

        create_file(&new_file).unwrap();

        assert_eq!(fs::read_to_string(&new_file).unwrap(), "");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn creating_a_file_over_an_existing_path_errors_via_handle_request() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("note.txt");
        fs::write(&existing, "content").unwrap();

        let request = Request::CreateFile {
            path: existing.to_string_lossy().into_owned(),
        };

        assert!(matches!(handle_request(&request), Response::Error { .. }));
        assert_eq!(fs::read_to_string(&existing).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deletes_exactly_the_given_files_and_directories() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(dir.join("sub")).unwrap();
        let file = dir.join("a.txt");
        let kept = dir.join("b.txt");
        fs::write(&file, "a").unwrap();
        fs::write(&kept, "b").unwrap();

        delete(&[
            file.to_string_lossy().into_owned(),
            dir.join("sub").to_string_lossy().into_owned(),
        ])
        .unwrap();

        assert!(!file.exists());
        assert!(!dir.join("sub").exists());
        assert!(kept.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracts_an_archive_via_the_archive_plugins_operation() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let archive_path = dir.join("test.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"payload").unwrap();
        writer.finish().unwrap();
        let destination = dir.join("out");

        extract(&archive_path, &destination).unwrap();

        assert_eq!(
            fs::read_to_string(destination.join("inside.txt")).unwrap(),
            "payload"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn journal_to_appends_a_line_describing_the_outcome() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let journal_path = dir.join("journal.jsonl");

        journal_to(
            &journal_path,
            "rename",
            &["a.txt".to_owned(), "b.txt".to_owned()],
            &Ok(()),
        );
        journal_to(
            &journal_path,
            "delete",
            &["c.txt".to_owned()],
            &Err(io::Error::other("boom")),
        );

        let contents = fs::read_to_string(&journal_path).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["operation"], "rename");
        assert_eq!(first["outcome"], "ok");
        assert_eq!(first["targets"], serde_json::json!(["a.txt", "b.txt"]));

        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["operation"], "delete");
        assert_eq!(second["outcome"], "boom");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reports_an_error_for_a_missing_directory() {
        let missing = std::env::temp_dir().join(unique_socket_name());
        let request = Request::ListDirectory {
            path: missing.to_string_lossy().into_owned(),
        };

        assert!(matches!(handle_request(&request), Response::Error { .. }));
    }

    #[test]
    fn answers_a_list_directory_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("file.txt"), b"").unwrap();

        let name = unique_socket_name();
        let listener = bind(name.as_str().to_ns_name::<GenericNamespaced>().unwrap()).unwrap();

        let dir_for_client = dir.clone();
        let client_name = name.clone();
        let client = std::thread::spawn(move || {
            let mut conn = Stream::connect(
                client_name
                    .as_str()
                    .to_ns_name::<GenericNamespaced>()
                    .unwrap(),
            )
            .unwrap();
            protocol::write_message(
                &mut conn,
                &Request::ListDirectory {
                    path: dir_for_client.to_string_lossy().into_owned(),
                },
            )
            .unwrap();
            protocol::read_message::<Response, _>(&mut conn).unwrap()
        });

        serve_one(&listener).unwrap();
        let response = client.join().unwrap();

        match response {
            Response::Directory { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].name, "file.txt");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Binds a listener on a fresh unique socket name and returns it
    /// alongside that name, for a test to connect a client to.
    fn bind_unique() -> (interprocess::local_socket::Listener, String) {
        let name = unique_socket_name();
        let listener = bind(name.as_str().to_ns_name::<GenericNamespaced>().unwrap()).unwrap();
        (listener, name)
    }

    fn connect(name: &str) -> Stream {
        Stream::connect(name.to_ns_name::<GenericNamespaced>().unwrap()).unwrap()
    }

    /// Sends `request` over a fresh socket, serves exactly one response to
    /// it, and returns what the client received - exercising the same
    /// wire path (`protocol::write_message`/`read_message` over a real
    /// local socket) that every front end actually uses, rather than
    /// calling [`handle_request`] in-process.
    fn round_trip(request: Request) -> Response {
        let (listener, name) = bind_unique();
        let client = std::thread::spawn(move || {
            let mut conn = connect(&name);
            protocol::write_message(&mut conn, &request).unwrap();
            protocol::read_message::<Response, _>(&mut conn).unwrap()
        });
        serve_one(&listener).unwrap();
        client.join().unwrap()
    }

    #[test]
    fn answers_a_view_file_request_over_the_socket() {
        let path = std::env::temp_dir().join(unique_socket_name());
        fs::write(&path, "hello over the wire").unwrap();

        let response = round_trip(Request::ViewFile {
            path: path.to_string_lossy().into_owned(),
        });

        match response {
            Response::FileView { plugin, data, .. } => {
                assert_eq!(plugin, "text");
                assert_eq!(data["content"], "hello over the wire");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn answers_a_rename_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("old.txt");
        let to = dir.join("new.txt");
        fs::write(&from, "content").unwrap();

        let response = round_trip(Request::Rename {
            from: from.to_string_lossy().into_owned(),
            to: to.to_string_lossy().into_owned(),
        });

        assert_eq!(response, Response::Done);
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn answers_a_copy_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("source.txt");
        let to = dir.join("copy.txt");
        fs::write(&from, "content").unwrap();

        let response = round_trip(Request::Copy {
            from: from.to_string_lossy().into_owned(),
            to: to.to_string_lossy().into_owned(),
        });

        assert_eq!(response, Response::Done);
        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn answers_a_delete_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let doomed = dir.join("doomed.txt");
        let kept = dir.join("kept.txt");
        fs::write(&doomed, "a").unwrap();
        fs::write(&kept, "b").unwrap();

        let response = round_trip(Request::Delete {
            paths: vec![doomed.to_string_lossy().into_owned()],
        });

        assert_eq!(response, Response::Done);
        assert!(!doomed.exists());
        assert!(kept.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn answers_an_extract_request_over_the_socket() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let archive_path = dir.join("test.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file("inside.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, b"payload").unwrap();
        writer.finish().unwrap();
        let destination = dir.join("out");

        let response = round_trip(Request::Extract {
            archive: archive_path.to_string_lossy().into_owned(),
            destination: destination.to_string_lossy().into_owned(),
        });

        assert_eq!(response, Response::Done);
        assert_eq!(
            fs::read_to_string(destination.join("inside.txt")).unwrap(),
            "payload"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn answers_an_error_over_the_socket_for_a_failed_operation() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        let from = dir.join("missing.txt");
        let to = dir.join("wherever.txt");

        let response = round_trip(Request::Rename {
            from: from.to_string_lossy().into_owned(),
            to: to.to_string_lossy().into_owned(),
        });

        assert!(matches!(response, Response::Error { .. }));
    }
    /// Two folder plugins that both recognise anything, so the collecting
    /// rule has something to collect. A registry with one real entry in it
    /// cannot tell "all of them" from "the first of them".
    struct AlwaysOne;
    struct AlwaysTwo;

    impl FolderCore for AlwaysOne {
        fn name(&self) -> &'static str {
            "always-one"
        }
        fn sniff(&self, _entries: &[&str]) -> bool {
            true
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "from": "one" }))
        }
    }

    impl FolderCore for AlwaysTwo {
        fn name(&self) -> &'static str {
            "always-two"
        }
        fn sniff(&self, _entries: &[&str]) -> bool {
            true
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::json!({ "from": "two" }))
        }
    }

    /// One that never matches, so filtering is shown to filter.
    struct NeverAny;

    impl FolderCore for NeverAny {
        fn name(&self) -> &'static str {
            "never-any"
        }
        fn sniff(&self, _entries: &[&str]) -> bool {
            false
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    #[test]
    fn every_matching_folder_plugin_contributes_not_just_the_first() {
        // A folder is several things at once - this repository's own root
        // is a source control working copy and a Cargo workspace - so the
        // dispatch collects. Replace the `filter` with a `find` and this
        // is the test that fails.
        let plugins: &[&'static dyn FolderCore] = &[&AlwaysOne, &NeverAny, &AlwaysTwo];

        let found = folder_plugins_among(plugins, &["Cargo.toml"]);

        assert_eq!(
            found.iter().map(|plugin| plugin.name()).collect::<Vec<_>>(),
            vec!["always-one", "always-two"],
            "both matching plugins should be collected, in registration order"
        );
    }

    #[test]
    fn a_folder_no_plugin_recognises_collects_nothing() {
        let plugins: &[&'static dyn FolderCore] = &[&NeverAny];

        assert!(folder_plugins_among(plugins, &["Cargo.toml"]).is_empty());
    }

    #[test]
    fn this_repository_is_a_working_copy_and_a_cargo_workspace_at_once() {
        // The case the whole design exists for. The folder answers as the
        // directory plugin, and carries the Cargo project beside it -
        // neither description replacing the other.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

        let response = view_file(&root).unwrap();

        let Response::FileView { plugin, also, .. } = response else {
            panic!("a folder should view as a file view");
        };
        assert_eq!(plugin, "directory", "the folder is still a folder");
        assert!(
            also.iter().any(|view| view.plugin == "project-cargo"),
            "and it is also a Cargo workspace: {:?}",
            also.iter().map(|view| &view.plugin).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_folder_that_is_no_kind_of_project_carries_nothing_extra() {
        let dir = std::env::temp_dir().join(format!("rse-plain-folder-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("notes.txt"), b"nothing to build here").unwrap();

        let response = view_file(&dir).unwrap();

        let Response::FileView { also, .. } = response else {
            panic!("a folder should view as a file view");
        };
        assert!(also.is_empty(), "a plain folder gains no project lines");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_never_carries_folder_views() {
        let file = std::env::temp_dir().join(format!("rse-plain-file-{}.txt", std::process::id()));
        std::fs::write(&file, b"a file has exactly one type").unwrap();

        let response = view_file(&file).unwrap();

        let Response::FileView { also, .. } = response else {
            panic!("a text file should view as a file view");
        };
        assert!(
            also.is_empty(),
            "only a folder can be several things at once"
        );

        std::fs::remove_file(&file).unwrap();
    }
    /// A general format that owns the extension.
    struct General;
    /// A narrower reading of the same format.
    struct Special;
    /// A sibling that overlaps but refines nothing, which is the case the
    /// extension hint was added for.
    struct Sibling;

    macro_rules! stub {
        ($ty:ty, $name:literal, $exts:expr, $refines:expr) => {
            impl PluginCore for $ty {
                fn name(&self) -> &'static str {
                    $name
                }
                fn extensions(&self) -> &'static [&'static str] {
                    $exts
                }
                fn specialises(&self) -> &'static [&'static str] {
                    $refines
                }
                fn sniff(&self, _prefix: &[u8]) -> bool {
                    true
                }
                fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
                    Ok(serde_json::Value::Null)
                }
            }
        };
    }
    stub!(General, "general", &["gen"], &[]);
    stub!(Special, "special", &[], &["general"]);
    stub!(Sibling, "sibling", &[], &[]);

    #[test]
    fn a_specialisation_beats_the_plugin_that_owns_the_extension() {
        // Without the drop, `claimed_by_extension` hands `a.gen` to
        // `general` however the list is ordered, because it owns `gen`.
        let plugins: &[&dyn PluginCore] = &[&Special, &General];

        let chosen = sniff_among(plugins, Path::new("a.gen"), b"anything");

        assert_eq!(chosen.map(PluginCore::name), Some("special"));
    }

    #[test]
    fn a_sibling_that_refines_nothing_does_not_displace_the_extension_owner() {
        // The hint still settles a genuine tie between siblings, which is
        // what a C file opening as Rust needed (#272).
        let plugins: &[&dyn PluginCore] = &[&Sibling, &General];

        let chosen = sniff_among(plugins, Path::new("a.gen"), b"anything");

        assert_eq!(chosen.map(PluginCore::name), Some("general"));
    }

    #[test]
    fn a_plugin_nothing_refines_is_kept() {
        let kept = most_specific(&[&Sibling, &General]);

        assert_eq!(kept.len(), 2, "neither refines the other");
    }

    #[test]
    fn an_unrelated_earlier_match_is_not_promoted_by_setting_the_general_one_aside() {
        // Merely dropping the refined plugin sent a Kubernetes manifest to
        // `sql`, which reads `---` as a comment: with `yaml` gone, `sql`
        // was simply first. The specialisation has to survive, not just
        // the general plugin disappear.
        let plugins: &[&dyn PluginCore] = &[&Sibling, &Special, &General];

        let chosen = sniff_among(plugins, Path::new("a.gen"), b"anything");

        assert_eq!(chosen.map(PluginCore::name), Some("special"));
    }
}
