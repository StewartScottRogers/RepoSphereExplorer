//! The fat process: filesystem traversal, indexing, operations, and plugin cores.

mod all_repositories;
pub mod repos;

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{Listener, ListenerOptions, Name, Stream};
use plugin_api::{FolderCore, PluginCore};
use protocol::{DirectoryEntry, NameMatch, PluginView, Request, Response, WorkingTreeSummary};
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
    &plugin_gradle::GradleCore,
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
    &plugin_npmmanifest::NpmmanifestCore,
    &plugin_devcontainer::DevcontainerCore,
    &plugin_webmanifest::WebmanifestCore,
    &plugin_sourcemap::SourcemapCore,
    &plugin_jsonschema::JsonschemaCore,
    &plugin_openapi::OpenapiCore,
    &plugin_cloudformation::CloudformationCore,
    &plugin_jsonlines::JsonlinesCore,
    &plugin_json5::Json5Core,
    &plugin_composerlock::ComposerlockCore,
    &plugin_json::JsonCore,
    &plugin_terraform::TerraformCore,
    &plugin_editorconfig::EditorconfigCore,
    &plugin_cargolock::CargolockCore,
    &plugin_pythonlock::PythonlockCore,
    &plugin_pyproject::PyprojectCore,
    &plugin_toml::TomlCore,
    &plugin_csv::CsvCore,
    &plugin_msgpack::MsgpackCore,
    &plugin_certificate::CertificateCore,
    &plugin_markdown::MarkdownCore,
    &plugin_helmchart::HelmchartCore,
    &plugin_gitlabci::GitlabciCore,
    &plugin_githubactions::GithubactionsCore,
    &plugin_kubernetes::KubernetesCore,
    &plugin_compose::ComposeCore,
    &plugin_yarnlock::YarnlockCore,
    &plugin_pnpmlock::PnpmlockCore,
    &plugin_ansible::AnsibleCore,
    &plugin_yaml::YamlCore,
    &plugin_gitmodules::GitmodulesCore,
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
    &plugin_cbor::CborCore,
    &plugin_bson::BsonCore,
    &plugin_arrow::ArrowCore,
    &plugin_orc::OrcCore,
    &plugin_apkpkg::ApkpkgCore,
    &plugin_gzip::GzipCore,
    &plugin_rubygem::RubygemCore,
    &plugin_tar::TarCore,
    &plugin_pickle::PickleCore,
    &plugin_bzip2::Bzip2Core,
    &plugin_zstd::ZstdCore,
    &plugin_xz::XzCore,
    &plugin_sevenzip::SevenzipCore,
    &plugin_javaclass::JavaclassCore,
    &plugin_pyc::PycCore,
    &plugin_minidump::MinidumpCore,
    &plugin_pcap::PcapCore,
    &plugin_sass::SassCore,
    &plugin_css::CssCore,
    &plugin_bicep::BicepCore,
    &plugin_nix::NixCore,
    &plugin_starlark::StarlarkCore,
    &plugin_cmake::CmakeCore,
    &plugin_meson::MesonCore,
    &plugin_ninja::NinjaCore,
    &plugin_cue::CueCore,
    &plugin_rego::RegoCore,
    &plugin_gemfilelock::GemfilelockCore,
    &plugin_gomod::GomodCore,
    &plugin_gosum::GosumCore,
    &plugin_text::TextCore,
    &plugin_image::ImageCore,
    &plugin_psd::PsdCore,
    &plugin_font::FontCore,
    &plugin_dotnetassembly::DotnetassemblyCore,
    &plugin_executable::ExecutableCore,
    &plugin_wasm::WasmCore,
    &plugin_word_document::WordDocumentCore,
    &plugin_spreadsheet::SpreadsheetCore,
    &plugin_presentation::PresentationCore,
    &plugin_epub::EpubCore,
    &plugin_comic_archive::ComicArchiveCore,
    &plugin_video::VideoCore,
    &plugin_audio::AudioCore,
    &plugin_numpy::NumpyCore,
    &plugin_jar::JarCore,
    &plugin_apk::ApkCore,
    &plugin_wheel::WheelCore,
    &plugin_nuget::NugetCore,
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
const FOLDER_PLUGINS: &[&dyn FolderCore] = &[
    &plugin_project_cargo::CargoProjectCore,
    &plugin_project_node::NodeProjectCore,
];

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

/// The most matches one search sends, whatever the request asks for.
pub const MAX_FIND_NAMES: usize = 500;

/// Every file and folder under `root` whose name contains `query`, ignoring
/// case, stopping at `limit`; and whether there were more than that.
///
/// Walks what `git` would see: `.gitignore`, `.ignore` and global exclude
/// rules apply, `.git` is never entered, and symbolic links are not
/// followed, so the walk cannot leave `root`. Ignore files above `root` are
/// not read - the Repos Directory is where the search starts and ends.
/// Hidden files are searched: `.github` and `.env` are things a reader
/// looks for.
///
/// Split from the request handler, which resolves `root` as the active
/// Repos Directory, so it can be tested without touching the machine's
/// configuration.
#[must_use]
pub fn find_names(root: &Path, query: &str, limit: usize) -> (Vec<NameMatch>, bool) {
    let wanted = query.trim().to_lowercase();
    if wanted.is_empty() {
        return (Vec::new(), false);
    }
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();

    let mut working_copies = std::collections::HashMap::new();
    let mut matches = Vec::new();
    // An unreadable folder is skipped, not the end of the search.
    for entry in walk.flatten() {
        if entry.depth() == 0
            || !entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains(&wanted)
        {
            continue;
        }
        if matches.len() == limit {
            return (matches, true);
        }
        let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
        let repository = nearest_repository(root, entry.path(), is_dir, &mut working_copies);
        matches.push(NameMatch {
            path: relative_to(root, entry.path()),
            is_dir,
            repository,
        });
    }
    (matches, false)
}

/// The nearest working copy holding `path` under `root` - `path` itself
/// when it is one - as a path relative to `root`, `/`-separated. `None`
/// when no working copy holds it.
///
/// `working_copies` memoizes [`repos::describe`] across one walk, since
/// several entries under the same folder ask the same question of it.
/// Shared by [`find_names`] and [`find_certificates`], which walk the same
/// way and attribute matches to a repository the same way.
fn nearest_repository(
    root: &Path,
    path: &Path,
    is_dir: bool,
    working_copies: &mut std::collections::HashMap<PathBuf, bool>,
) -> Option<String> {
    let nearest = if is_dir { Some(path) } else { path.parent() };
    nearest
        .into_iter()
        .flat_map(Path::ancestors)
        .take_while(|folder| folder.starts_with(root))
        .find(|folder| {
            *working_copies
                .entry(folder.to_path_buf())
                .or_insert_with(|| repos::describe(folder).is_some())
        })
        .map(|folder| relative_to(root, folder))
}

/// `path` relative to `root`, with `/` between its components on every
/// platform, as [`NameMatch`] carries it.
pub(crate) fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Reads at most `limit` bytes from the start of the file at `path`.
fn read_capped(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Reads a bounded prefix from the start of the file at `path`, for
/// sniffing its type.
fn read_prefix(path: &Path) -> io::Result<Vec<u8>> {
    read_capped(path, SNIFF_PREFIX_LEN)
}

/// Largest prefix read from a candidate certificate/key file while
/// [`find_certificates`] scans it (#621): a PEM chain plus its key rarely
/// exceeds a few kilobytes, but a mis-tagged multi-megabyte binary file
/// must not be read whole.
const CERTIFICATE_SCAN_PREFIX: u64 = 1024 * 1024;

/// Every certificate, private key and certificate signing request
/// committed under `root` (#621), read-only (D10, rule 8).
///
/// Walked with the same rules [`find_names`] uses: no `.git`, unreadable
/// folders skipped, not following symbolic links. Only files whose
/// extension the certificate plugin claims are opened, and only their
/// first [`CERTIFICATE_SCAN_PREFIX`] bytes are read.
#[must_use]
pub fn find_certificates(root: &Path) -> Vec<protocol::CertificateFinding> {
    let walk = ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .follow_links(false)
        .sort_by_file_name(std::cmp::Ord::cmp)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build();

    let mut working_copies = std::collections::HashMap::new();
    let mut findings = Vec::new();
    for entry in walk.flatten() {
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            continue;
        }
        let path = entry.path();
        let claimed = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|extension| plugin_certificate::EXTENSIONS.contains(&extension.as_str()));
        if !claimed {
            continue;
        }
        let kind = read_capped(path, CERTIFICATE_SCAN_PREFIX)
            .ok()
            .and_then(|bytes| plugin_certificate::scan(&bytes).ok())
            .filter(|blocks| !blocks.is_empty())
            .map_or(protocol::CertificateFindingKind::Unreadable, |blocks| {
                protocol::CertificateFindingKind::Blocks(
                    blocks.into_iter().map(certificate_block).collect(),
                )
            });
        findings.push(protocol::CertificateFinding {
            path: relative_to(root, path),
            repository: nearest_repository(root, path, false, &mut working_copies),
            kind,
        });
    }
    findings
}

/// Converts one plugin-side [`plugin_certificate::ScannedBlock`] into the
/// wire type [`find_certificates`] sends.
fn certificate_block(block: plugin_certificate::ScannedBlock) -> protocol::CertificateBlock {
    match block {
        plugin_certificate::ScannedBlock::Certificate(summary) => {
            protocol::CertificateBlock::Certificate(protocol::CertificateSummary {
                subject: summary.subject,
                issuer: summary.issuer,
                serial: summary.serial,
                not_before: summary.not_before,
                not_after: summary.not_after,
                self_signed: summary.self_signed,
            })
        }
        plugin_certificate::ScannedBlock::PrivateKey => protocol::CertificateBlock::PrivateKey,
        plugin_certificate::ScannedBlock::CertificateRequest => {
            protocol::CertificateBlock::CertificateRequest
        }
        plugin_certificate::ScannedBlock::Unreadable => protocol::CertificateBlock::Unreadable,
    }
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
    let metadata = fs::metadata(path)
        .map_err(|err| io::Error::new(err.kind(), format!("{}: {err}", path.display())))?;
    if metadata.is_dir() {
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
                    data: with_source_text(path, data?),
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

/// Largest file whose text is carried in a view. The same ceiling the
/// plugins that read text already use, so a file is treated the same way
/// whichever of them opens it.
const MAX_SOURCE_BYTES: u64 = 64 * 1024;

/// Puts the file's own text into `data` when the plugin that read it did
/// not.
///
/// A `content` string in a view is what makes the front end offer the raw
/// Text tab and the editor. Until now that depended on whether a plugin's
/// author had kept the source: fifty-two of them parse a text format
/// thoroughly and throw the text away, so a `.css`, a `.zig` or a
/// `CMakeLists.txt` could be read *about* but never read or edited. Being
/// well supported was what made a file uneditable.
///
/// It belongs here rather than in each plugin. This process is the only
/// one that touches the filesystem, it has already opened the file, and
/// the guarantee wanted is about the file rather than about its format:
/// **every view of a text file carries that file's text**.
///
/// A plugin that kept its own `content` keeps it, untouched. A file that
/// is not valid text, or is past [`MAX_SOURCE_BYTES`], gets nothing - and
/// a view with no text is what tells the front end not to offer an editor
/// it could only save half a file from.
fn with_source_text(path: &Path, mut data: serde_json::Value) -> serde_json::Value {
    let Some(object) = data.as_object_mut() else {
        return data;
    };
    if object.contains_key("content") {
        return data;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return data;
    };
    if metadata.len() > MAX_SOURCE_BYTES {
        return data;
    }
    let Ok(bytes) = fs::read(path) else {
        return data;
    };
    let Ok(text) = String::from_utf8(bytes) else {
        return data;
    };
    object.insert("content".to_owned(), serde_json::Value::String(text));
    // Only when the plugin said nothing: a plugin that reports its own
    // reading was cut short keeps saying so, and the front end goes on
    // refusing to edit what it has only part of.
    object
        .entry("truncated")
        .or_insert(serde_json::Value::Bool(false));
    data
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
    /// The steps [`Request::Undo`] would take. GUIDANCE.md §2 keeps
    /// business rules out of the front ends, so the service remembers what
    /// the last operation was and how to reverse it; a front end only asks.
    ///
    /// One *operation* deep, per D6, which settles "undo of the immediately
    /// preceding operation" and "batch operations" in the same breath: a
    /// paste of three files is one thing the reader did, so it is one thing
    /// Ctrl+Z puts back. Several steps, reversed in reverse order, rather
    /// than several undos.
    ///
    /// Thread-local because the service answers every request on one thread
    /// ([`run`] loops over [`serve_one`]), which also keeps the tests from
    /// treading on each other's steps.
    static UNDO: std::cell::RefCell<Vec<Undoable>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Records `steps` as what would undo the operation just performed. An
/// empty list clears the record, for an operation that cannot be undone.
fn remember_undo(steps: Vec<Undoable>) {
    UNDO.with_borrow_mut(|slot| *slot = steps);
}

/// Records an undo step only if `result` succeeded; a failed operation
/// changed nothing and leaves the previous step alone.
fn remember_if_done(result: &io::Result<()>, step: Undoable) {
    if result.is_ok() {
        remember_undo(vec![step]);
    }
}

/// Undoes the last operation, journaling the attempt.
///
/// # Errors
/// Returns an error if there is nothing to undo, or if reversing it fails.
pub fn undo() -> io::Result<()> {
    let steps = UNDO.with_borrow_mut(std::mem::take);
    if steps.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "nothing to undo. A delete is undone from the Recycle Bin",
        ));
    }
    // Newest first: a batch that created a folder and then filled it has
    // to be emptied before the folder goes, and the same argument holds
    // for any two steps that touched the same place. `pop` takes the
    // newest, so what is left in `steps` is always what has not been
    // tried.
    let mut steps = steps;
    while let Some(step) = steps.pop() {
        if let Err(err) = undo_step(step) {
            // The steps this undo never reached are still owed. Taking
            // the journal and then leaving on the first failure dropped
            // them: a three-file move where one old name had been
            // re-created by hand put one file back, refused the second,
            // and silently forgot the third - half undone, with no way
            // to finish it.
            remember_undo(steps);
            return Err(err);
        }
    }
    Ok(())
}

/// Reverses one recorded step, journaling the attempt.
fn undo_step(step: Undoable) -> io::Result<()> {
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
pub fn rename(items: &[(String, String)]) -> io::Result<()> {
    batch("rename", items, |from, to| {
        refuse_if_occupied_by_another(from, to)?;
        fs::rename(from, to)?;
        Ok(Undoable::Rename {
            from: to.to_path_buf(),
            to: from.to_path_buf(),
        })
    })
}

/// Copies the file at `from` to `to`, journaling the attempt. Refuses to
/// replace an existing `to`, for the same reason [`rename`] does.
///
/// # Errors
/// Returns an error if something already exists at `to`, or if the copy
/// fails.
pub fn copy(items: &[(String, String)]) -> io::Result<()> {
    batch("copy", items, |from, to| {
        refuse_if_exists(to)?;
        fs::copy(from, to)?;
        Ok(Undoable::Remove {
            path: to.to_path_buf(),
        })
    })
}

/// Runs `act` over every pair, stopping at the first failure, and records
/// what actually landed as the one operation `Undo` reverses.
///
/// The steps are remembered even when the batch failed part way through.
/// A reader who pastes three files and has the second refused is left with
/// one file they did not have before; leaving that un-undoable would make
/// the mess permanent, and Ctrl+Z is exactly what they would reach for.
fn batch(
    operation: &str,
    items: &[(String, String)],
    mut act: impl FnMut(&Path, &Path) -> io::Result<Undoable>,
) -> io::Result<()> {
    let mut steps = Vec::new();
    let mut targets = Vec::new();
    let result = (|| {
        for (from, to) in items {
            let (from, to) = (Path::new(from), Path::new(to));
            targets.push(from.display().to_string());
            targets.push(to.display().to_string());
            steps.push(act(from, to)?);
        }
        Ok(())
    })();
    // Only when something actually landed. An operation refused before
    // it touched anything must not cost the reader the undo of the
    // operation before it - rename a file, then attempt a paste that is
    // refused because the name is taken, and the paste did nothing at all
    // while Ctrl+Z could no longer put the rename back. `remember_if_done`
    // states the same rule for the single-step operations.
    if !steps.is_empty() {
        remember_undo(steps);
    }
    journal(operation, &targets, &result);
    result
}

/// Writes `content` to `path` by writing a temporary file beside it and
/// renaming it into place, so an interrupted write leaves the original
/// intact rather than a half-written file.
fn write_atomically(path: &Path, content: &str) -> io::Result<()> {
    // The suffix carries this process and a count, so the temporary is a
    // name nobody else holds. A fixed `.rse-write` was written with
    // `fs::write`, which replaces whatever it finds: a file the reader had
    // made and named themselves was erased outright - not to the recycle
    // bin - and undoing the edit did not bring it back. Refusing the save
    // instead would have been worse, stopping them saving their own file
    // because of an unrelated sibling. It also means two saves of the same
    // file can no longer collide on one temporary.
    static NEXT_TEMPORARY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let ticket = NEXT_TEMPORARY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".rse-write-{}-{ticket}", std::process::id()));
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
        remember_undo(vec![Undoable::Restore {
            path: path.to_path_buf(),
            content: previous,
        }]);
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
    remember_undo(Vec::new());
    journal("delete", paths, &result);
    result
}

/// Extracts the archive at `archive` into `destination`, journaling the
/// attempt.
///
/// # Errors
/// Returns an error if the archive cannot be extracted.
pub fn extract(archive: &Path, destination: &Path) -> io::Result<()> {
    let result = plugin_archive::extract(archive, destination).map(|created| {
        // One step per path the extraction made, and nothing it found
        // there - see #521. On success the journal is replaced even when
        // that list is empty, because an extraction that only overwrote
        // existing files still changed them, and leaving the previous
        // operation's step would have Ctrl+Z reach past it.
        remember_undo(
            created
                .into_iter()
                .map(|path| Undoable::Remove { path })
                .collect(),
        );
    });
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

/// Whether the working copy at `path` has uncommitted changes to its
/// tracked files, or `None` when `path` is not a working copy or its index
/// could not be read.
///
/// One pass over one checkout's tracked files, read and never driven (D10).
#[must_use]
pub fn working_tree_status(path: &Path) -> Option<WorkingTreeSummary> {
    let status = plugin_directory::repository::describe_with_status(path)?.status?;
    Some(WorkingTreeSummary {
        changed: status.changed,
        partial: status.partial,
        summary: status.summary(),
    })
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
        Request::Rename { items } => respond_to_operation(rename(items)),
        Request::Copy { items } => respond_to_operation(copy(items)),
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
        Request::WorkingTreeStatus { path } => Response::WorkingTree {
            path: path.clone(),
            status: working_tree_status(Path::new(path)),
        },
        Request::FindNames { query, limit } => match repos::active_root() {
            Some(root) => {
                // The front end asks for what it wants to show, but the
                // service decides what it will walk and send: a limit is a
                // number anybody on the socket can make enormous.
                let limit = (*limit).min(MAX_FIND_NAMES);
                let (matches, cut_short) = find_names(&root, query, limit);
                Response::Names {
                    root: root.to_string_lossy().into_owned(),
                    matches,
                    cut_short,
                }
            }
            None => Response::Error {
                message: "no Repos Directory is configured to search".to_owned(),
            },
        },
        Request::SetReposRoot { path } => {
            let target = Path::new(path);
            let outcome = repos::set_active_root(target);
            journal("set-repos-root", &[target.display().to_string()], &outcome);
            respond_to_operation(outcome)
        }
        Request::AllRepositories { root, refresh } => {
            let (entries, done) = all_repositories::poll(Path::new(root), *refresh);
            Response::AllRepositories { entries, done }
        }
        Request::FindCertificates => match repos::active_root() {
            Some(root) => Response::Certificates {
                certificates: find_certificates(&root),
                complete: true,
            },
            None => Response::Error {
                message: "no Repos Directory is configured to search".to_owned(),
            },
        },
    }
}

/// Starts listening on the local socket identified by `name`.
///
/// # Errors
/// Returns an error if the socket is already in use or cannot be created.
pub fn bind(name: Name<'_>) -> io::Result<Listener> {
    ListenerOptions::new().name(name).create_sync()
}

/// As [`bind`], but takes over a socket file that a service killed without
/// the chance to clean up has left behind.
///
/// Where local sockets are files - macOS - a service ended by a signal
/// leaves its socket file in place. Every later service then failed to bind
/// with "address in use" and exited, so a front end could neither reach one
/// nor start one, and the application would not open again until somebody
/// deleted a file in the temporary directory by hand.
///
/// The file is replaced only when nothing answers on it. A service that is
/// actually running keeps its socket, and this returns the "in use" error
/// as [`bind`] does. Named pipes and namespaced sockets leave nothing
/// behind, so elsewhere this is [`bind`].
///
/// # Errors
/// As [`bind`], including when a live service already holds the socket.
pub fn bind_reclaiming_stale(name: Name<'_>) -> io::Result<Listener> {
    use interprocess::local_socket::traits::Stream as _;

    match bind(name.borrow()) {
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            if Stream::connect(name.borrow()).is_ok() {
                return Err(err);
            }
            ListenerOptions::new()
                .name(name)
                .try_overwrite(true)
                .create_sync()
        }
        result => result,
    }
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
    if let Err(err) = protocol::write_message(&mut conn, &response) {
        // A response the front end could not read is worse than no
        // response: dropping the connection leaves the reader with a
        // generic failure, or nothing at all. The one thing still worth
        // sending is why. A plugin that hands back something too deeply
        // nested to travel is the case this exists for.
        if err.kind() != io::ErrorKind::InvalidData {
            return Err(err);
        }
        let excuse = Response::Error {
            message: format!("this file cannot be shown: {err}"),
        };
        journal("response_refused", &[format!("{err}")], &Err(err));
        return protocol::write_message(&mut conn, &excuse);
    }
    Ok(())
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
        CORE_PLUGINS, FolderCore, MAX_SOURCE_BYTES, Path, bind, copy, create_directory,
        create_file, delete, extract, find_certificates, find_names, folder_plugins_among, guarded,
        handle_request, journal_to, list_directory, most_specific, open, rename, repos, serve_one,
        sniff_among, undo, view_file, with_source_text, working_tree_status, write_atomically,
        write_file,
    };
    use interprocess::local_socket::traits::Stream as _;
    use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
    use plugin_api::PluginCore;

    /// One source-and-destination pair, for the tests that move a single
    /// file through an interface that now takes a batch.
    fn one(from: &Path, to: &Path) -> Vec<(String, String)> {
        vec![(from.display().to_string(), to.display().to_string())]
    }
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

        rename(&one(&from, &to)).unwrap();

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

        copy(&one(&from, &to)).unwrap();

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

        let err = rename(&one(&from, &to)).unwrap_err();

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

        let err = copy(&one(&from, &to)).unwrap_err();

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

        rename(&one(&path, &path)).unwrap();

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

    /// D6 settles "batch operations" and "undo of the immediately
    /// preceding operation" in the same breath, so a move of several files
    /// is one operation and one undo puts all of it back.
    #[test]
    fn one_undo_reverses_a_whole_batch_rename() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let names = ["one", "two", "three"];
        let items: Vec<(String, String)> = names
            .iter()
            .map(|name| {
                let from = dir.join(format!("{name}.txt"));
                fs::write(&from, *name).unwrap();
                (
                    from.display().to_string(),
                    dir.join(format!("{name}.moved")).display().to_string(),
                )
            })
            .collect();

        rename(&items).unwrap();
        for name in names {
            assert!(dir.join(format!("{name}.moved")).exists());
            assert!(!dir.join(format!("{name}.txt")).exists());
        }

        undo().unwrap();

        for name in names {
            assert!(
                dir.join(format!("{name}.txt")).exists(),
                "{name}.txt should have come back from one undo"
            );
            assert!(!dir.join(format!("{name}.moved")).exists());
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    /// A batch that fails part way still records what it managed, so the
    /// reader can put it back.
    ///
    /// Copying three files with the second refused leaves one file they
    /// did not have before. Remembering nothing - which is what recording
    /// only a wholly successful operation would do - would make that
    /// permanent, and Ctrl+Z is exactly what a reader reaches for when an
    /// operation reports a failure.
    #[test]
    fn a_batch_that_fails_part_way_can_still_be_undone() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.txt");
        let second = dir.join("second.txt");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        let landed = dir.join("landed.txt");
        // The second destination already exists, so `copy` refuses it.
        let occupied = dir.join("occupied.txt");
        fs::write(&occupied, "in the way").unwrap();

        let err = copy(&[
            (first.display().to_string(), landed.display().to_string()),
            (second.display().to_string(), occupied.display().to_string()),
        ])
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(landed.exists(), "the first copy did land");
        assert_eq!(fs::read_to_string(&occupied).unwrap(), "in the way");

        undo().unwrap();

        assert!(
            !landed.exists(),
            "the half of the batch that landed has to be undoable, or a \
             failed paste leaves a mess that cannot be cleared"
        );
        assert_eq!(
            fs::read_to_string(&occupied).unwrap(),
            "in the way",
            "and the file that was in the way is not the batch's to touch"
        );
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_puts_a_renamed_file_back() {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        let from = dir.join("before.txt");
        let to = dir.join("after.txt");
        fs::write(&from, "content").unwrap();
        rename(&one(&from, &to)).unwrap();

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
        copy(&one(&from, &to)).unwrap();

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
    /// The far end of the editor's save: the bytes actually land.
    ///
    /// The graphical front end's own tests stop at the text a save would
    /// carry, because the save crosses a process boundary by design.
    /// This is the other side of it.
    #[test]
    fn writing_a_file_puts_the_bytes_on_disk() {
        let directory = std::env::temp_dir().join("repos-explorer-write-file");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        let path = directory.join("demo.rs");
        std::fs::write(&path, "before\n").expect("the fixture is written");

        let response = round_trip(Request::WriteFile {
            path: path.to_string_lossy().into_owned(),
            content: "// hifn main() {}\n".to_owned(),
        });

        assert!(
            matches!(response, Response::Done),
            "the service says it wrote it: {response:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file is still there"),
            "// hifn main() {}\n",
            "and it wrote what it was given, over what was there"
        );
    }

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
            items: one(&from, &to),
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
            items: one(&from, &to),
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
            items: one(&from, &to),
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

    // ---------------------------------------------------------------
    // The error paths, which is where a reader's files get lost.
    //
    // Every test below works inside a directory of its own under the
    // system temporary directory and removes it afterwards; nothing here
    // writes outside one, and the only paths given to `delete` - which
    // uses the real recycle bin - are scratch paths.
    //
    // The undo journal is a single thread-local slot. The test harness
    // gives each test its own thread, so each starts with an empty
    // journal and none of them can tread on another's steps: that is what
    // lets these run in parallel without serialising, and why a test may
    // call `undo` first thing and expect "nothing to undo".
    // ---------------------------------------------------------------

    /// A fresh, uniquely named scratch directory.
    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(unique_socket_name());
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The names directly inside `dir`, sorted, for asserting on what a
    /// failed operation did or did not leave behind.
    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Writes a one-entry zip at `path`, for the extract tests.
    fn write_zip(path: &Path, entry: &str, payload: &[u8]) {
        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(entry, zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut writer, payload).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn undo_with_an_empty_journal_points_at_the_recycle_bin_rather_than_failing_silently() {
        // Nothing has happened on this thread, so this is the cold start a
        // reader meets when they press Ctrl+Z first thing.
        let err = undo().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("Recycle Bin"),
            "the one thing a reader can still recover has to be named: {err}"
        );
    }

    #[test]
    fn undoing_twice_does_not_reach_back_past_the_last_operation() {
        // D6: one operation deep. The second Ctrl+Z must not quietly take
        // apart work the reader did before the one they meant to undo.
        let dir = scratch();
        let first = dir.join("first");
        let second = dir.join("second");
        create_directory(&first).unwrap();
        create_directory(&second).unwrap();

        undo().unwrap();
        let err = undo().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!second.exists(), "the last operation is the one undone");
        assert!(
            first.is_dir(),
            "and the operation before it is not reached back to"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    /// FINDING: a batch refused before it touches anything still clears
    /// the journal, so the operation before it stops being undoable.
    ///
    /// `batch` calls `remember_undo(steps)` unconditionally, and `steps`
    /// is empty when the very first pair is refused. Its sibling
    /// `remember_if_done` says the opposite in so many words - "a failed
    /// operation changed nothing and leaves the previous step alone" - and
    /// `a_failed_operation_leaves_the_previous_undo_step_alone` pins that
    /// for `create_directory`. A reader who renames a file and then has a
    /// paste refused loses the rename's undo to an operation that did
    /// nothing at all.
    #[test]
    fn a_batch_refused_at_its_first_step_leaves_the_previous_undo_step_alone() {
        let dir = scratch();
        let created = dir.join("kept");
        create_directory(&created).unwrap();
        let source = dir.join("source.txt");
        fs::write(&source, "content").unwrap();
        let occupied = dir.join("occupied.txt");
        fs::write(&occupied, "in the way").unwrap();

        let err = copy(&one(&source, &occupied)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(&occupied).unwrap(),
            "in the way",
            "the refused copy changed nothing whatsoever"
        );

        let undone = undo();
        let created_still_there = created.exists();
        fs::remove_dir_all(&dir).unwrap();

        undone.expect(
            "an operation that changed nothing must not cost the reader the \
             undo of the operation before it",
        );
        assert!(
            !created_still_there,
            "the create is what should have undone"
        );
    }

    #[test]
    fn a_batch_refused_at_its_middle_step_undoes_the_one_that_landed() {
        let dir = scratch();
        for name in ["one", "two", "three"] {
            fs::write(dir.join(format!("{name}.txt")), name).unwrap();
        }
        let occupied = dir.join("two.copy");
        fs::write(&occupied, "in the way").unwrap();
        let pairs: Vec<(String, String)> = ["one", "two", "three"]
            .iter()
            .map(|name| {
                (
                    dir.join(format!("{name}.txt")).display().to_string(),
                    dir.join(format!("{name}.copy")).display().to_string(),
                )
            })
            .collect();

        let err = copy(&pairs).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(dir.join("one.copy").exists(), "the first pair landed");
        assert!(
            !dir.join("three.copy").exists(),
            "and the batch stopped rather than carrying on past the refusal"
        );

        undo().unwrap();

        assert!(
            !dir.join("one.copy").exists(),
            "the half that landed clears"
        );
        assert_eq!(
            fs::read_to_string(&occupied).unwrap(),
            "in the way",
            "and the file that was in the way is not the batch's to touch"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_refused_at_its_last_step_undoes_both_that_landed() {
        let dir = scratch();
        for name in ["one", "two", "three"] {
            fs::write(dir.join(format!("{name}.txt")), name).unwrap();
        }
        fs::write(dir.join("three.copy"), "in the way").unwrap();
        let pairs: Vec<(String, String)> = ["one", "two", "three"]
            .iter()
            .map(|name| {
                (
                    dir.join(format!("{name}.txt")).display().to_string(),
                    dir.join(format!("{name}.copy")).display().to_string(),
                )
            })
            .collect();

        copy(&pairs).unwrap_err();
        assert!(dir.join("one.copy").exists());
        assert!(dir.join("two.copy").exists());

        undo().unwrap();

        assert!(!dir.join("one.copy").exists());
        assert!(!dir.join("two.copy").exists());
        assert_eq!(
            fs::read_to_string(dir.join("three.copy")).unwrap(),
            "in the way"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_batch_undoes_its_steps_newest_first_so_a_chain_comes_apart() {
        // Two renames in a chain: `a` becomes `b`, then that `b` becomes
        // `c`. Reversed oldest-first, the first step would look for a `b`
        // that is now `c` and fail; reversed newest-first it unwinds.
        let dir = scratch();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let c = dir.join("c.txt");
        fs::write(&a, "content").unwrap();

        rename(&[
            (a.display().to_string(), b.display().to_string()),
            (b.display().to_string(), c.display().to_string()),
        ])
        .unwrap();
        assert_eq!(names_in(&dir), vec!["c.txt".to_owned()]);

        undo().unwrap();

        assert_eq!(
            names_in(&dir),
            vec!["a.txt".to_owned()],
            "one undo unwinds the whole chain, oldest step last"
        );
        assert_eq!(fs::read_to_string(&a).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// FINDING: an undo that fails part way throws away the steps it never
    /// took.
    ///
    /// `undo` takes the whole journal out of the slot with `mem::take` and
    /// then returns on the first failing step. Everything after that point
    /// is dropped, and a second Ctrl+Z reports "nothing to undo" - so a
    /// three-file move that a reader partly re-made by hand becomes half
    /// undone and permanently so.
    #[test]
    fn an_undo_that_fails_part_way_keeps_the_steps_it_has_not_taken() {
        let dir = scratch();
        let names = ["one", "two", "three"];
        let pairs: Vec<(String, String)> = names
            .iter()
            .map(|name| {
                let from = dir.join(format!("{name}.txt"));
                fs::write(&from, *name).unwrap();
                (
                    from.display().to_string(),
                    dir.join(format!("{name}.moved")).display().to_string(),
                )
            })
            .collect();
        rename(&pairs).unwrap();

        // The reader puts a file back at the middle step's old name, so
        // that step's undo is refused rather than allowed to replace it.
        fs::write(dir.join("two.txt"), "put back by hand").unwrap();

        let first = undo().unwrap_err();
        assert_eq!(first.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            dir.join("three.txt").exists(),
            "the newest step was taken before the refusal"
        );
        assert!(
            dir.join("one.moved").exists(),
            "and the oldest step was never reached"
        );

        let second = undo();
        let one_is_back = dir.join("one.txt").exists();
        fs::remove_dir_all(&dir).unwrap();

        second.expect("the step the failed undo never took is still owed to the reader");
        assert!(one_is_back, "and taking it puts the last file back");
    }

    #[test]
    fn undoing_a_rename_whose_file_has_since_gone_reports_it_rather_than_pretending() {
        let dir = scratch();
        let from = dir.join("before.txt");
        let to = dir.join("after.txt");
        fs::write(&from, "content").unwrap();
        rename(&one(&from, &to)).unwrap();
        // Removed underneath the service, the way any other program can.
        fs::remove_file(&to).unwrap();

        let err = undo().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            !from.exists(),
            "and nothing is conjured at the old name to cover it up"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_a_rename_refuses_to_replace_something_put_back_at_the_old_name() {
        let dir = scratch();
        let from = dir.join("before.txt");
        let to = dir.join("after.txt");
        fs::write(&from, "original").unwrap();
        rename(&one(&from, &to)).unwrap();
        fs::write(&from, "a different file the reader made").unwrap();

        let err = undo().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(&from).unwrap(),
            "a different file the reader made",
            "an undo is not a licence to destroy what has arrived since"
        );
        assert_eq!(fs::read_to_string(&to).unwrap(), "original");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undoing_an_edit_writes_the_old_text_back_even_where_the_file_has_gone() {
        // Pins what `Undoable::Restore` does when the file it describes is
        // no longer there: it recreates it. The old text is the reader's
        // work, and the alternative is losing it to a deletion made
        // somewhere else entirely.
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "before").unwrap();
        write_file(&path, "after").unwrap();
        fs::remove_file(&path).unwrap();

        undo().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "before");
        assert_eq!(
            names_in(&dir),
            vec!["notes.txt".to_owned()],
            "and no temporary is left beside it"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_delete_that_fails_still_clears_the_journal_rather_than_undoing_something_else() {
        // Deliberate, and the comment on `delete` says why: the service
        // cannot pull anything back out of the recycle bin, so a stale
        // step here would put back the wrong thing.
        let dir = scratch();
        let created = dir.join("kept");
        create_directory(&created).unwrap();

        let err = delete(&[dir.join("never-existed.txt").display().to_string()]).unwrap_err();
        assert!(!err.to_string().is_empty());

        let err = undo().unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            created.is_dir(),
            "and the create is left standing, not undone"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_delete_that_stops_part_way_leaves_nothing_to_undo() {
        let dir = scratch();
        let doomed = dir.join("doomed.txt");
        fs::write(&doomed, "content").unwrap();

        let err = delete(&[
            doomed.display().to_string(),
            dir.join("never-existed.txt").display().to_string(),
        ])
        .unwrap_err();

        assert!(!err.to_string().is_empty());
        assert!(!doomed.exists(), "the first path did go to the recycle bin");
        assert_eq!(
            undo().unwrap_err().kind(),
            io::ErrorKind::NotFound,
            "and the recycle bin is the only way back for it"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_extract_of_something_that_is_not_an_archive_leaves_the_previous_undo_step_alone() {
        let dir = scratch();
        let created = dir.join("kept");
        create_directory(&created).unwrap();
        let not_an_archive = dir.join("prose.txt");
        fs::write(&not_an_archive, "this is not a zip file").unwrap();
        let destination = dir.join("out");

        extract(&not_an_archive, &destination).unwrap_err();

        assert!(
            !destination.exists(),
            "a failed extract leaves no empty directory behind"
        );

        undo().unwrap();

        assert!(!created.exists(), "the create before it is still undoable");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// FINDING: undoing an extract removes the whole destination folder,
    /// including whatever was already in it.
    ///
    /// `extract` records `Undoable::Remove { path: destination }`, whose
    /// documented meaning is "remove what an operation created". But
    /// `plugin_archive::extract` calls `create_dir_all` and does not
    /// refuse an existing destination, so extracting into a folder a
    /// reader already had and then pressing Ctrl+Z sends that folder - all
    /// of it - to the recycle bin. Recoverable from there, and still not
    /// what undo promised.
    #[test]
    fn undoing_an_extract_must_not_remove_what_was_already_in_the_destination() {
        let dir = scratch();
        let archive_path = dir.join("test.zip");
        write_zip(&archive_path, "inside.txt", b"payload");
        let destination = dir.join("out");
        fs::create_dir_all(&destination).unwrap();
        let keepsake = destination.join("keepsake.txt");
        fs::write(&keepsake, "the reader's own file").unwrap();

        extract(&archive_path, &destination).unwrap();
        assert!(destination.join("inside.txt").exists());

        undo().unwrap();

        let survived = fs::read_to_string(&keepsake);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(
            survived.ok().as_deref(),
            Some("the reader's own file"),
            "undo removes what the operation created, not what it found there"
        );
    }

    #[test]
    fn a_refused_write_leaves_the_previous_undo_step_alone() {
        let dir = scratch();
        let created = dir.join("kept");
        create_directory(&created).unwrap();

        write_file(&dir.join("never-existed.txt"), "text").unwrap_err();

        undo().unwrap();

        assert!(!created.exists());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_file_to_another_case_of_its_own_name_is_the_rename_it_reads_as() {
        // The pre-filled rename prompt makes "readme.txt" -> "README.txt"
        // an ordinary thing to type. On a case-insensitive filesystem the
        // destination "already exists" - it is the same file - so this is
        // exactly the case `refuse_if_occupied_by_another` is for.
        let dir = scratch();
        let lower = dir.join("readme.txt");
        let upper = dir.join("README.txt");
        fs::write(&lower, "content").unwrap();

        rename(&one(&lower, &upper)).unwrap();

        assert_eq!(
            names_in(&dir),
            vec!["README.txt".to_owned()],
            "the capitalisation the reader typed is the name on disk"
        );
        assert_eq!(fs::read_to_string(&upper).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_onto_a_sibling_that_differs_only_in_case_does_not_destroy_it() {
        let dir = scratch();
        let keep = dir.join("keep.txt");
        let moving = dir.join("moving.txt");
        fs::write(&keep, "precious").unwrap();
        fs::write(&moving, "content").unwrap();

        let outcome = rename(&one(&moving, &dir.join("KEEP.txt")));

        if cfg!(windows) {
            assert_eq!(
                outcome.unwrap_err().kind(),
                io::ErrorKind::AlreadyExists,
                "where the filesystem conflates the two names they are one name"
            );
            assert_eq!(fs::read_to_string(&keep).unwrap(), "precious");
            assert_eq!(fs::read_to_string(&moving).unwrap(), "content");
        } else {
            outcome.unwrap();
            assert_eq!(
                fs::read_to_string(&keep).unwrap(),
                "precious",
                "and where it does not, they are two names and neither is touched"
            );
            assert_eq!(fs::read_to_string(dir.join("KEEP.txt")).unwrap(), "content");
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_onto_an_existing_directory_is_refused_rather_than_attempted() {
        let dir = scratch();
        let from = dir.join("note.txt");
        let occupied = dir.join("occupied");
        fs::write(&from, "content").unwrap();
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("inside.txt"), "a whole folder of work").unwrap();

        let err = rename(&one(&from, &occupied)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read_to_string(occupied.join("inside.txt")).unwrap(),
            "a whole folder of work"
        );
        assert_eq!(fs::read_to_string(&from).unwrap(), "content");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copying_onto_an_existing_directory_is_refused_rather_than_attempted() {
        let dir = scratch();
        let from = dir.join("note.txt");
        let occupied = dir.join("occupied");
        fs::write(&from, "content").unwrap();
        fs::create_dir_all(&occupied).unwrap();

        let err = copy(&one(&from, &occupied)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(occupied.is_dir());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn copying_a_file_onto_itself_is_refused_rather_than_emptying_it() {
        // `fs::copy` with the same source and destination truncates the
        // file to nothing on some platforms, so the refusal is the whole
        // protection here.
        let dir = scratch();
        let path = dir.join("only-copy.txt");
        fs::write(&path, "irreplaceable").unwrap();

        let err = copy(&one(&path, &path)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "irreplaceable");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_into_a_folder_that_does_not_exist_leaves_the_source_where_it_was() {
        let dir = scratch();
        let from = dir.join("note.txt");
        fs::write(&from, "content").unwrap();

        let err = rename(&one(&from, &dir.join("no-such-folder").join("note.txt"))).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            names_in(&dir),
            vec!["note.txt".to_owned()],
            "and no folder is conjured on the way"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_source_that_is_not_there_reports_not_found_and_creates_nothing() {
        let dir = scratch();

        let err = rename(&one(&dir.join("gone.txt"), &dir.join("wherever.txt"))).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(names_in(&dir).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_refusal_names_the_path_that_is_in_the_way() {
        // The front ends show this message verbatim; a reader cannot act
        // on "already exists" without being told what does.
        let dir = scratch();
        let from = dir.join("note.txt");
        let occupied = dir.join("occupied.txt");
        fs::write(&from, "content").unwrap();
        fs::write(&occupied, "precious").unwrap();

        let message = rename(&one(&from, &occupied)).unwrap_err().to_string();

        assert!(
            message.contains("occupied.txt"),
            "the message has to name the obstacle: {message}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_folder_onto_an_occupied_name_leaves_both_folders_alone() {
        let dir = scratch();
        let from = dir.join("source");
        let occupied = dir.join("occupied");
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("mine.txt"), "mine").unwrap();
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("theirs.txt"), "theirs").unwrap();

        let err = rename(&one(&from, &occupied)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(from.join("mine.txt")).unwrap(), "mine");
        assert_eq!(
            fs::read_to_string(occupied.join("theirs.txt")).unwrap(),
            "theirs"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Makes `link` a symbolic link to `target`, or reports that this
    /// platform will not let an unprivileged process make one - Windows
    /// without Developer Mode.
    fn try_symlink_file(target: &Path, link: &Path) -> bool {
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(target, link);
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        made.is_ok()
    }

    #[test]
    fn a_link_pointing_nowhere_still_occupies_the_name_it_sits_at() {
        // `refuse_if_exists` asks `symlink_metadata` rather than
        // `metadata` for exactly this: a link whose target has gone is
        // still a thing at that path, and renaming onto it would destroy
        // it without a word.
        let dir = scratch();
        let dangling = dir.join("dangling");
        if !try_symlink_file(&dir.join("never-existed.txt"), &dangling) {
            // No privilege to make one here; the rule is unproven rather
            // than disproven.
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        let from = dir.join("note.txt");
        fs::write(&from, "content").unwrap();

        let err = rename(&one(&from, &dangling)).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            dangling.symlink_metadata().is_ok(),
            "the link is still there"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dangling_link_is_listed_rather_than_failing_the_whole_directory() {
        // One unreadable entry must not cost a reader the sight of the
        // folder it sits in.
        let dir = scratch();
        if !try_symlink_file(&dir.join("never-existed.txt"), &dir.join("dangling")) {
            fs::remove_dir_all(&dir).unwrap();
            return;
        }
        fs::write(dir.join("readable.txt"), "content").unwrap();

        let entries = list_directory(&dir).unwrap();

        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            vec!["dangling", "readable.txt"]
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_zero_byte_write_empties_the_file_and_the_old_text_still_comes_back() {
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "work worth keeping").unwrap();

        write_file(&path, "").unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert_eq!(
            names_in(&dir),
            vec!["notes.txt".to_owned()],
            "an empty write is still one rename, not a leftover temporary"
        );

        undo().unwrap();

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "work worth keeping",
            "emptying a file is the most destructive edit there is, so it undoes"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_over_a_read_only_file_leaves_it_wholly_old_or_wholly_new_and_no_temporary() {
        // Which of the two depends on the platform - replacing a file is a
        // directory operation on Unix and a file operation on Windows -
        // but the guarantee `write_atomically` exists for holds either
        // way: never half a file, and never a `.rse-write` orphan.
        let dir = scratch();
        let path = dir.join("locked.txt");
        fs::write(&path, "before").unwrap();
        let writable = fs::metadata(&path).unwrap().permissions();
        let mut read_only = writable.clone();
        read_only.set_readonly(true);
        fs::set_permissions(&path, read_only).unwrap();

        let outcome = write_file(&path, "after");

        let text = fs::read_to_string(&path).unwrap();
        assert!(
            text == "before" || text == "after",
            "never a half-written file: {text:?}"
        );
        assert_eq!(
            outcome.is_ok(),
            text == "after",
            "and what it reported is what happened"
        );
        assert_eq!(
            names_in(&dir),
            vec!["locked.txt".to_owned()],
            "no temporary orphaned beside it"
        );

        // Put back exactly the permissions the file had, so the scratch
        // directory can be cleared up.
        fs::set_permissions(&path, writable).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_atomically_onto_a_directory_fails_and_takes_its_temporary_with_it() {
        let dir = scratch();
        let occupied = dir.join("a-folder");
        fs::create_dir_all(&occupied).unwrap();
        fs::write(occupied.join("inside.txt"), "content").unwrap();

        write_atomically(&occupied, "text").unwrap_err();

        assert_eq!(
            names_in(&dir),
            vec!["a-folder".to_owned()],
            "the temporary it wrote first is cleared up when the rename fails"
        );
        assert_eq!(
            fs::read_to_string(occupied.join("inside.txt")).unwrap(),
            "content"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    /// FINDING: a save destroys any file that happens to sit at
    /// `<name>.rse-write`.
    ///
    /// `write_atomically` writes its temporary with `fs::write`, which
    /// replaces whatever is there, and then renames it away. Every other
    /// destination in this crate is guarded by `refuse_if_exists` for
    /// precisely this reason - "an operation that would silently replace
    /// it stops before touching the filesystem" - and this one is not. The
    /// file is gone outright rather than to the recycle bin, and undo of
    /// the edit does not bring it back.
    #[test]
    fn a_save_must_not_destroy_a_sibling_named_after_its_temporary_file() {
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "before").unwrap();
        let sibling = dir.join("notes.txt.rse-write");
        fs::write(&sibling, "a file the reader made and named themselves").unwrap();

        write_file(&path, "after").unwrap();

        let survived = fs::read_to_string(&sibling);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            survived.ok().as_deref(),
            Some("a file the reader made and named themselves"),
            "a save may replace the file it was given and nothing else"
        );
    }

    #[test]
    fn writing_a_file_that_is_not_valid_text_is_refused_before_its_bytes_are_touched() {
        // The old text is read first so the edit can be undone. A file
        // that is not text has no old text to read, and going ahead would
        // write an edit that could never be taken back.
        let dir = scratch();
        let path = dir.join("picture.bin");
        let bytes: &[u8] = &[0xFF, 0xFE, 0x00, 0x01, 0x80];
        fs::write(&path, bytes).unwrap();

        write_file(&path, "text").unwrap_err();

        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(names_in(&dir), vec!["picture.bin".to_owned()]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn writing_to_a_path_whose_parent_does_not_exist_is_refused() {
        let dir = scratch();

        let err = write_file(&dir.join("no-such-folder").join("notes.txt"), "text").unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(names_in(&dir).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_far_larger_than_a_buffer_lands_whole_and_undoes_whole() {
        let dir = scratch();
        let path = dir.join("big.txt");
        let before = "old line\n".repeat(20_000);
        let after = "new line\n".repeat(30_000);
        fs::write(&path, &before).unwrap();

        write_file(&path, &after).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), after);
        assert_eq!(names_in(&dir), vec!["big.txt".to_owned()]);

        undo().unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_directory_lists_as_no_entries_rather_than_an_error() {
        let dir = scratch();

        assert!(list_directory(&dir).unwrap().is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_a_path_that_is_a_file_is_an_error_not_an_empty_listing() {
        // An empty listing would tell a reader their folder was empty.
        let dir = scratch();
        let file = dir.join("note.txt");
        fs::write(&file, "content").unwrap();

        assert!(list_directory(&file).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listing_reads_a_working_copy_out_of_its_own_files_and_never_runs_git() {
        // D10: detect, do not drive. The marker, the branch and the remote
        // are all file reads, which is why this fixture - a `.git`
        // directory nobody ever ran `git init` in - is enough.
        let dir = scratch();
        let checkout = dir.join("widgets");
        let git = checkout.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/acme/widgets.git\n",
        )
        .unwrap();

        let entries = list_directory(&dir).unwrap();

        let entry = entries.iter().find(|e| e.name == "widgets").unwrap();
        let found = entry
            .repository
            .as_ref()
            .expect("a checkout, by its marker");
        assert_eq!(found.branch.as_deref(), Some("main"));
        assert_eq!(found.provider.as_deref(), Some("github.com"));
        assert_eq!(
            found.remote.as_deref(),
            Some("https://github.com/acme/widgets.git")
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listing_carries_a_worktrees_kind_and_the_clone_it_shares() {
        // D10 again: a `commondir` file beside a bare marker is enough to
        // tell a linked worktree apart from an ordinary clone, and no `git`
        // command is run to find out.
        let dir = scratch();
        let clone_git = dir.join("clone").join(".git");
        fs::create_dir_all(&clone_git).unwrap();
        fs::write(clone_git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            clone_git.join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/acme/widgets.git\n",
        )
        .unwrap();

        let worktree_git = clone_git.join("worktrees").join("side");
        fs::create_dir_all(&worktree_git).unwrap();
        fs::write(worktree_git.join("HEAD"), "ref: refs/heads/side\n").unwrap();
        fs::write(worktree_git.join("commondir"), "../..\n").unwrap();

        let linked = dir.join("linked");
        fs::create_dir_all(&linked).unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", worktree_git.display()),
        )
        .unwrap();

        let entries = list_directory(&dir).unwrap();

        let entry = entries.iter().find(|e| e.name == "linked").unwrap();
        let found = entry
            .repository
            .as_ref()
            .expect("a worktree, by its marker");
        assert_eq!(
            found.kind,
            protocol::RepositoryKind::Worktree {
                clone: dir.join("clone").to_string_lossy().into_owned(),
                clone_exists: true,
            }
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listing_carries_a_repositorys_last_activity() {
        // The folder's own modification time only moves when an entry
        // directly inside it changes; last activity is read from the
        // checkout's own files instead (#588).
        let dir = scratch();
        let checkout = dir.join("widgets");
        let git = checkout.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let entries = list_directory(&dir).unwrap();

        let entry = entries.iter().find(|e| e.name == "widgets").unwrap();
        let found = entry
            .repository
            .as_ref()
            .expect("a checkout, by its marker");
        let expected = fs::metadata(git.join("HEAD"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(
            found.last_activity,
            Some(expected),
            "HEAD is the only one of the three activity files this fixture wrote"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_listing_carries_a_repositorys_last_fetch() {
        // Across a Repos Directory, nothing showed which checkouts had not
        // been fetched in months without opening each one; the listing
        // reports FETCH_HEAD's own modification time for every row instead
        // (#589).
        let dir = scratch();
        let checkout = dir.join("widgets");
        let git = checkout.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("FETCH_HEAD"), b"").unwrap();

        let entries = list_directory(&dir).unwrap();

        let entry = entries.iter().find(|e| e.name == "widgets").unwrap();
        let found = entry
            .repository
            .as_ref()
            .expect("a checkout, by its marker");
        let expected = fs::metadata(git.join("FETCH_HEAD"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(found.last_fetch, Some(expected));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn listing_two_hundred_repositories_stays_a_stat_per_row() {
        // GUIDANCE.md 3.4/3.5: a directory read per row would make a
        // listing of this size crawl. Reading last activity and last fetch
        // is a stat of a handful of named files per repository (CLAUDE.md
        // rule 9), never a read of the git directory's contents - so 200 of
        // them stays fast. The actual wall-clock time is reported in the
        // pull request alongside the same listing's time before this
        // change.
        let dir = scratch();
        for n in 0..200 {
            let git = dir.join(format!("repo-{n}")).join(".git");
            fs::create_dir_all(git.join("logs")).unwrap();
            fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(
                git.join("config"),
                "[remote \"origin\"]\n\turl = https://github.com/acme/widgets.git\n",
            )
            .unwrap();
            fs::write(git.join("index"), b"").unwrap();
            fs::write(git.join("logs").join("HEAD"), b"").unwrap();
            fs::write(git.join("FETCH_HEAD"), b"").unwrap();
        }

        let start = std::time::Instant::now();
        let entries = list_directory(&dir).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(entries.len(), 200);
        for entry in &entries {
            let found = entry
                .repository
                .as_ref()
                .unwrap_or_else(|| panic!("{} should be a working copy", entry.name));
            assert!(
                found.last_activity.is_some(),
                "{} should have last activity",
                entry.name
            );
            assert!(
                found.last_fetch.is_some(),
                "{} should have a last fetch (#589)",
                entry.name
            );
        }
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "200 repositories took {elapsed:?}, far more than a handful of stats per row should"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_ordinary_folder_is_listed_without_being_called_a_working_copy() {
        let dir = scratch();
        fs::create_dir_all(dir.join("just-a-folder")).unwrap();
        fs::write(dir.join("just-a-file.txt"), "content").unwrap();

        let entries = list_directory(&dir).unwrap();

        for entry in &entries {
            assert!(
                entry.repository.is_none(),
                "{} is no kind of checkout",
                entry.name
            );
        }
        assert_eq!(entries.len(), 2, "and both are still listed either way");

        fs::remove_dir_all(&dir).unwrap();
    }

    /// A file name that is not valid Unicode, made the way this platform
    /// allows one.
    #[cfg(windows)]
    fn invalid_unicode_name() -> std::ffi::OsString {
        use std::os::windows::ffi::OsStringExt as _;
        // An unpaired surrogate: legal in a Windows file name, and not
        // representable in UTF-8.
        std::ffi::OsString::from_wide(&[u16::from(b'r'), 0xD800, u16::from(b's')])
    }

    /// A file name that is not valid Unicode, made the way this platform
    /// allows one.
    #[cfg(unix)]
    fn invalid_unicode_name() -> std::ffi::OsString {
        use std::os::unix::ffi::OsStrExt as _;
        std::ffi::OsStr::from_bytes(b"r\xffs").to_owned()
    }

    #[test]
    fn a_name_that_is_not_valid_unicode_is_listed_but_cannot_be_acted_on() {
        // The wire carries names as `String`, so this one is lossy by the
        // time a front end sees it. The listing is honest about the file
        // being there; the name it hands back no longer reaches it, which
        // is the cost of the protocol and is pinned here so a change to it
        // is a deliberate one.
        let dir = scratch();
        let name = invalid_unicode_name();
        if fs::write(dir.join(&name), "content").is_err() {
            fs::remove_dir_all(&dir).unwrap();
            return;
        }

        let entries = list_directory(&dir).unwrap();

        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].name.contains('\u{FFFD}'),
            "the lossy name is what crosses the wire: {:?}",
            entries[0].name
        );
        assert!(
            !dir.join(&entries[0].name).exists(),
            "and it names nothing, so a rename of it would fail rather than \
             land on some other file"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_far_longer_than_any_filesystem_allows_is_an_error_not_a_panic() {
        let dir = scratch();
        let absurd = dir.join("n".repeat(400));

        assert!(create_file(&absurd).is_err());
        assert!(create_directory(&absurd).is_err());
        assert!(list_directory(&absurd).is_err());
        assert!(view_file(&absurd).is_err());
        assert!(names_in(&dir).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn viewing_a_file_that_is_not_there_is_an_error_rather_than_an_empty_view() {
        let dir = scratch();
        let path = dir.join("gone.txt");

        let err = view_file(&path).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "the error should name the file that is not there: {err}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_removed_after_being_viewed_names_itself_in_the_error() {
        let dir = scratch();
        let alpha = dir.join("alpha");
        fs::create_dir(&alpha).unwrap();
        assert!(view_file(&alpha).is_ok(), "a plain folder views fine");

        fs::remove_dir(&alpha).unwrap();
        let err = view_file(&alpha).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains(&alpha.display().to_string()),
            "the error should name the folder that vanished: {err}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_file_is_viewed_without_any_plugin_reading_past_its_end() {
        // Nothing to sniff is the smallest hostile input there is, and
        // every one of the registered plugins is offered it.
        let dir = scratch();
        let path = dir.join("empty.txt");
        fs::write(&path, b"").unwrap();

        let response = view_file(&path).unwrap();

        match response {
            Response::FileView { data, .. } => {
                assert_eq!(data["content"], "", "an empty file's text is empty");
            }
            Response::Error { message } => {
                assert!(message.contains("empty.txt"), "{message}");
            }
            other => panic!("unexpected response: {other:?}"),
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_view_that_already_carries_its_own_text_keeps_it_untouched() {
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "what is on disk").unwrap();

        let data = with_source_text(
            &path,
            serde_json::json!({ "content": "what the plugin kept" }),
        );

        assert_eq!(data["content"], "what the plugin kept");
        assert!(
            data.get("truncated").is_none(),
            "and nothing else is put in beside it"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_view_that_is_not_an_object_is_handed_back_as_it_came() {
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "text").unwrap();

        assert_eq!(
            with_source_text(&path, serde_json::json!([1, 2, 3])),
            serde_json::json!([1, 2, 3])
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_text_file_under_the_ceiling_gains_its_text_and_is_marked_whole() {
        let dir = scratch();
        let path = dir.join("notes.txt");
        fs::write(&path, "line one\nline two\n").unwrap();

        let data = with_source_text(&path, serde_json::json!({ "lines": 2 }));

        assert_eq!(data["content"], "line one\nline two\n");
        assert_eq!(
            data["truncated"], false,
            "so the front end knows it may offer the editor"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_text_file_past_the_ceiling_carries_no_text_so_no_editor_is_offered() {
        // An editor that could only save back part of a file is how a
        // reader loses the rest of it.
        let dir = scratch();
        let path = dir.join("huge.txt");
        let oversized = "x".repeat(usize::try_from(MAX_SOURCE_BYTES).unwrap() + 1);
        fs::write(&path, &oversized).unwrap();

        let data = with_source_text(&path, serde_json::json!({ "lines": 1 }));

        assert!(data.get("content").is_none());
        assert!(data.get("truncated").is_none());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_is_not_valid_text_carries_no_text() {
        let dir = scratch();
        let path = dir.join("picture.bin");
        fs::write(&path, [0xFFu8, 0xFE, 0x00, 0x80]).unwrap();

        let data = with_source_text(&path, serde_json::json!({ "pixels": 1 }));

        assert!(
            data.get("content").is_none(),
            "half-decoded bytes are not this file's text"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_has_gone_between_the_view_and_the_text_carries_no_text() {
        let dir = scratch();

        let data = with_source_text(&dir.join("gone.txt"), serde_json::json!({ "lines": 0 }));

        assert!(data.get("content").is_none(), "and no panic on the way");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn most_specific_of_no_matches_is_no_matches() {
        assert!(most_specific(&[]).is_empty());
    }

    /// Two plugins that each claim to refine the other - a registry
    /// mistake, not a shape the design intends.
    struct EachWay;
    /// The other half of the pair.
    struct OtherWay;
    stub!(EachWay, "each-way", &[], &["other-way"]);
    stub!(OtherWay, "other-way", &[], &["each-way"]);

    #[test]
    fn two_plugins_that_each_refine_the_other_do_not_cancel_each_other_out() {
        // A cycle in `specialises` must not leave a file with no plugin at
        // all: the reader would see "no plugin recognises" for a file two
        // plugins recognised.
        let kept = most_specific(&[&EachWay, &OtherWay]);

        assert_eq!(
            kept.len(),
            2,
            "neither is dropped, so priority order settles it"
        );

        let chosen = sniff_among(&[&EachWay, &OtherWay], Path::new("a.txt"), b"anything");
        assert_eq!(chosen.map(PluginCore::name), Some("each-way"));
    }

    #[test]
    fn a_plugin_refining_one_that_did_not_match_is_not_treated_as_more_specific() {
        // `Special` refines `general`, which is not among the matches
        // here, so it has refined nothing and must not displace the
        // extension's owner.
        let chosen = sniff_among(&[&Sibling, &Special], Path::new("a.gen"), b"anything");

        assert_eq!(
            chosen.map(PluginCore::name),
            Some("sibling"),
            "priority order, because neither claims `gen` and neither refines the other"
        );
    }

    #[test]
    fn an_uppercase_extension_still_settles_a_tie() {
        // Windows hands back `README.GEN` as readily as `readme.gen`, and
        // the hint is documented as lowercase.
        let chosen = sniff_among(&[&Sibling, &General], Path::new("A.GEN"), b"anything");

        assert_eq!(chosen.map(PluginCore::name), Some("general"));
    }

    #[test]
    fn a_file_with_no_extension_at_all_falls_back_to_priority_order() {
        let chosen = sniff_among(&[&Sibling, &General], Path::new("Makefile"), b"anything");

        assert_eq!(chosen.map(PluginCore::name), Some("sibling"));
    }

    #[test]
    fn a_file_no_plugin_recognises_is_claimed_by_none_of_them() {
        let dir = scratch();
        let path = dir.join("sample.bin");
        fs::write(&path, b"anything").unwrap();

        assert!(
            sniff_among(&[], &path, b"anything").is_none(),
            "an empty registry claims nothing, rather than picking arbitrarily"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    /// How many times the counting folder plugin has been asked to sniff.
    static FOLDER_SNIFFS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// A folder plugin that records having been asked.
    struct Counting;

    impl FolderCore for Counting {
        fn name(&self) -> &'static str {
            "counting"
        }
        fn sniff(&self, _entries: &[&str]) -> bool {
            FOLDER_SNIFFS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            true
        }
        fn view(&self, _path: &Path) -> io::Result<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }
    }

    #[test]
    fn a_folder_plugin_is_asked_even_after_an_earlier_one_has_matched() {
        // D12 again, from the other side: short-circuiting on the first
        // match would cost the folder every description after it.
        let plugins: &[&'static dyn FolderCore] = &[&AlwaysOne, &Counting];
        FOLDER_SNIFFS.store(0, std::sync::atomic::Ordering::Relaxed);

        let found = folder_plugins_among(plugins, &["Cargo.toml"]);

        assert_eq!(found.len(), 2);
        assert_eq!(
            FOLDER_SNIFFS.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the plugin behind a match is still asked"
        );
    }

    #[test]
    fn a_folder_plugin_that_cannot_read_its_manifest_costs_only_its_own_lines() {
        let dir = scratch();
        fs::write(dir.join("Cargo.toml"), "this is not TOML at all {{{").unwrap();
        fs::write(dir.join("notes.txt"), "content").unwrap();

        let response = view_file(&dir).unwrap();

        let Response::FileView { plugin, data, also } = response else {
            panic!("a folder should view as a file view");
        };
        assert_eq!(plugin, "directory", "the folder is still a folder");
        assert_eq!(data["entry_count"], 2, "and still says what is inside it");
        assert!(
            also.is_empty(),
            "the project line is what is lost, and only that: {also:?}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_a_project_is_described_as_both_at_once() {
        let dir = scratch();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"widgets\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let Response::FileView { plugin, also, .. } = view_file(&dir).unwrap() else {
            panic!("a folder should view as a file view");
        };

        assert_eq!(plugin, "directory");
        assert!(
            also.iter().any(|view| view.plugin == "project-cargo"),
            "the project description is added, never substituted: {:?}",
            also.iter().map(|view| &view.plugin).collect::<Vec<_>>()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_folder_that_is_a_node_project_is_described_as_both_at_once() {
        let dir = scratch();
        fs::write(dir.join("package.json"), "{\"name\": \"widgets\"}").unwrap();

        let Response::FileView { plugin, also, .. } = view_file(&dir).unwrap() else {
            panic!("a folder should view as a file view");
        };

        assert_eq!(plugin, "directory", "the folder is still a folder");
        assert!(
            also.iter().any(|view| view.plugin == "project-node"),
            "the project description is added, never substituted: {:?}",
            also.iter().map(|view| &view.plugin).collect::<Vec<_>>()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_node_and_a_cargo_project_at_once_both_contribute() {
        // A folder is several things at once (D12): a monorepo root can
        // hold both a `package.json` for its tooling and a `Cargo.toml`
        // for a Rust component, and neither should crowd the other out.
        let dir = scratch();
        fs::write(dir.join("package.json"), "{\"name\": \"widgets\"}").unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"widgets\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();

        let Response::FileView { also, .. } = view_file(&dir).unwrap() else {
            panic!("a folder should view as a file view");
        };

        let plugins: Vec<&str> = also.iter().map(|view| view.plugin.as_str()).collect();
        assert!(plugins.contains(&"project-node"));
        assert!(plugins.contains(&"project-cargo"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_panic_carrying_no_string_still_names_the_plugin_and_the_file() {
        let dir = scratch();
        let path = dir.join("trips-a-parser.bin");
        fs::write(&path, b"anything").unwrap();

        let outcome: Result<(), io::Error> = guarded("odd", &path, || std::panic::panic_any(7_u32));

        let message = outcome.unwrap_err().to_string();
        assert!(message.contains("odd"), "{message}");
        assert!(message.contains("trips-a-parser.bin"), "{message}");
        assert!(
            message.contains("no message"),
            "a payload nothing can read still has to be reported: {message}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_path_is_an_error_for_every_request_that_takes_one() {
        let hostile = [
            Request::ListDirectory {
                path: String::new(),
            },
            Request::ViewFile {
                path: String::new(),
            },
            Request::Open {
                path: String::new(),
            },
            Request::CreateDirectory {
                path: String::new(),
            },
            Request::CreateFile {
                path: String::new(),
            },
            Request::WriteFile {
                path: String::new(),
                content: "text".to_owned(),
            },
            Request::Rename {
                items: vec![(String::new(), String::new())],
            },
            Request::Copy {
                items: vec![(String::new(), String::new())],
            },
            Request::Delete {
                paths: vec![String::new()],
            },
            Request::Extract {
                archive: String::new(),
                destination: String::new(),
            },
            Request::SetReposRoot {
                path: String::new(),
            },
        ];

        for request in hostile {
            assert!(
                matches!(handle_request(&request), Response::Error { .. }),
                "an empty path is not a path: {request:?}"
            );
        }
    }

    #[test]
    fn a_path_of_separators_alone_never_becomes_an_operation_on_the_filesystem_root() {
        // Only the separators this platform actually has. A backslash is a
        // legal character in a Unix filename, so `\` there is not the root
        // at all - it is a relative name, and asking to create it would
        // succeed and leave a directory called `\` in the working
        // directory rather than refusing anything.
        let separators: &[&str] = if cfg!(windows) {
            &["/", "\\", "//", "\\\\", "///"]
        } else {
            &["/", "//", "///"]
        };
        for path in separators {
            let path = *path;
            for request in [
                Request::CreateDirectory {
                    path: path.to_owned(),
                },
                Request::CreateFile {
                    path: path.to_owned(),
                },
                Request::WriteFile {
                    path: path.to_owned(),
                    content: "text".to_owned(),
                },
            ] {
                assert!(
                    matches!(handle_request(&request), Response::Error { .. }),
                    "the root of a volume is nothing to create or overwrite: {request:?}"
                );
            }
        }
    }

    #[test]
    fn a_relative_path_is_taken_as_written_rather_than_rejected() {
        // The soft boundary of D8: the service resolves what it is given
        // against its own working directory and does not police it.
        let Response::Directory { entries } = handle_request(&Request::ListDirectory {
            path: ".".to_owned(),
        }) else {
            panic!("a relative path names a real directory");
        };

        assert!(
            entries.iter().any(|entry| entry.name == "Cargo.toml"),
            "`.` is this crate while its tests run: {:?}",
            entries.iter().map(|entry| &entry.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_parent_traversal_out_of_a_folder_and_back_names_the_same_folder() {
        // D8 makes the Repos Directory boundary soft - one configuration
        // point, not a rule spread through the navigation code - so `..`
        // is resolved rather than refused. Pinned so that stops being an
        // accident of `Path` and becomes a decision.
        let dir = scratch();
        fs::create_dir_all(dir.join("sub")).unwrap();
        let direct = dir.join("marker.txt");
        fs::write(&direct, "the same file either way").unwrap();
        let roundabout = dir.join("sub").join("..").join("marker.txt");

        let Response::FileView { data, .. } = handle_request(&Request::ViewFile {
            path: roundabout.to_string_lossy().into_owned(),
        }) else {
            panic!("the traversal should reach the file");
        };

        assert_eq!(data["content"], "the same file either way");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opening_a_path_that_is_not_there_is_an_error() {
        let dir = scratch();

        assert!(matches!(
            handle_request(&Request::Open {
                path: dir.join("gone.txt").to_string_lossy().into_owned(),
            }),
            Response::Error { .. }
        ));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_undo_request_with_nothing_to_undo_answers_with_an_error_not_a_done() {
        // A `Done` here would tell a reader their last operation had been
        // reversed when nothing had happened.
        assert!(matches!(
            handle_request(&Request::Undo),
            Response::Error { .. }
        ));
    }

    #[test]
    fn asking_for_the_repos_roots_always_offers_somewhere_to_open_at() {
        // A first run has no stored roots, and the front ends still have
        // to open somewhere (D7).
        let Response::ReposRoots { default, .. } = handle_request(&Request::ReposRoots) else {
            panic!("the roots request has exactly one kind of answer");
        };

        assert!(!default.is_empty());
    }

    #[test]
    fn setting_a_repos_root_that_is_not_a_directory_is_refused_and_stores_nothing() {
        let dir = scratch();
        let file = dir.join("not-a-workspace.txt");
        fs::write(&file, "a file, not a folder").unwrap();
        let before = repos::roots();

        let response = handle_request(&Request::SetReposRoot {
            path: file.to_string_lossy().into_owned(),
        });

        assert!(matches!(response, Response::Error { .. }));
        assert_eq!(
            repos::roots(),
            before,
            "a refused request does not rewrite the machine's settings"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extracting_something_that_is_not_an_archive_leaves_no_destination_behind() {
        let dir = scratch();
        let prose = dir.join("prose.txt");
        fs::write(&prose, "this is not a zip file").unwrap();
        let destination = dir.join("out");

        let response = handle_request(&Request::Extract {
            archive: prose.to_string_lossy().into_owned(),
            destination: destination.to_string_lossy().into_owned(),
        });

        assert!(matches!(response, Response::Error { .. }));
        assert!(
            !destination.exists(),
            "an empty folder left behind is a folder the reader has to clear up"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renaming_a_file_into_a_subfolder_is_a_move_and_undoes_as_one() {
        let dir = scratch();
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        let from = dir.join("note.txt");
        let to = sub.join("note.txt");
        fs::write(&from, "content").unwrap();

        rename(&one(&from, &to)).unwrap();
        assert_eq!(fs::read_to_string(&to).unwrap(), "content");
        assert!(!from.exists());

        undo().unwrap();

        assert_eq!(fs::read_to_string(&from).unwrap(), "content");
        assert!(names_in(&sub).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    /// Extracting into a folder that did not exist, then undoing, removes
    /// it entirely - the behaviour that already held, still covered now
    /// that the journal records paths rather than the destination.
    #[test]
    fn undoing_an_extract_into_a_new_folder_removes_the_folder() {
        let dir = scratch();
        let archive_path = dir.join("test.zip");
        write_zip(&archive_path, "inside.txt", b"payload");
        let destination = dir.join("out");

        extract(&archive_path, &destination).unwrap();
        assert!(destination.join("inside.txt").exists());

        undo().unwrap();

        let gone = !destination.exists();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            gone,
            "the folder was made by the extraction, so undo takes it"
        );
    }

    /// An extraction that only overwrote existing files changed them, so
    /// Ctrl+Z must not reach past it to the operation before - but it has
    /// nothing it can put back either, and says so.
    #[test]
    fn an_extract_that_only_overwrote_files_leaves_nothing_to_undo_rather_than_the_step_before() {
        let dir = scratch();
        let earlier = dir.join("earlier");
        create_directory(&earlier).unwrap();

        let archive_path = dir.join("test.zip");
        write_zip(&archive_path, "inside.txt", b"new");
        let destination = dir.join("out");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("inside.txt"), "old").unwrap();

        extract(&archive_path, &destination).unwrap();
        let answer = undo();

        let earlier_survives = earlier.exists();
        let _ = fs::remove_dir_all(&dir);
        assert!(
            answer.is_err(),
            "there is nothing this extraction can put back"
        );
        assert!(
            earlier_survives,
            "and the folder created before it must not be undone in its place"
        );
    }

    /// A fake checkout at `root/name`: a `.git` directory with the two files
    /// a clone has, which is all the working copy marker needs. No `git`
    /// runs.
    fn checkout(root: &Path, name: &str) -> std::path::PathBuf {
        let dir = root.join(name);
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = https://github.com/owner/name.git\n",
        )
        .unwrap();
        dir
    }

    /// The paths of `find_names`' matches, in order.
    fn paths_of(matches: &[protocol::NameMatch]) -> Vec<&str> {
        matches.iter().map(|found| found.path.as_str()).collect()
    }

    #[test]
    fn finds_a_name_in_every_repository_and_says_which() {
        let root = scratch();
        fs::write(checkout(&root, "alpha").join("Cargo.toml"), "").unwrap();
        let beta_crate = checkout(&root, "beta").join("crates").join("core");
        fs::create_dir_all(&beta_crate).unwrap();
        fs::write(beta_crate.join("Cargo.toml"), "").unwrap();

        let (matches, cut_short) = find_names(&root, "Cargo.toml", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            paths_of(&matches),
            ["alpha/Cargo.toml", "beta/crates/core/Cargo.toml"]
        );
        assert_eq!(matches[0].repository.as_deref(), Some("alpha"));
        assert_eq!(matches[1].repository.as_deref(), Some("beta"));
        assert!(!cut_short, "two matches is everything");
    }

    #[test]
    fn a_nested_checkout_is_the_nearest_repository() {
        let root = scratch();
        let inner = checkout(&checkout(&root, "outer"), "inner");
        fs::write(inner.join("notes.md"), "").unwrap();

        let (matches, _) = find_names(&root, "notes", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(matches[0].repository.as_deref(), Some("outer/inner"));
    }

    #[test]
    fn a_file_a_gitignore_excludes_is_not_found() {
        let root = scratch();
        let repo = checkout(&root, "app");
        fs::write(repo.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::create_dir_all(repo.join("target").join("debug")).unwrap();
        fs::write(repo.join("target").join("debug").join("index.js"), "").unwrap();
        fs::write(repo.join("index.log"), "").unwrap();
        fs::write(repo.join("index.rs"), "").unwrap();

        let (matches, _) = find_names(&root, "index", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&matches), ["app/index.rs"]);
    }

    /// What the `ignore` crate does, pinned: like `git`, it honours a
    /// `.gitignore` only inside a working copy, while a `.ignore` applies
    /// anywhere.
    #[test]
    fn a_gitignore_outside_a_working_copy_does_not_apply_but_an_ignore_file_does() {
        let root = scratch();
        let loose = root.join("loose");
        fs::create_dir_all(&loose).unwrap();
        fs::write(loose.join(".gitignore"), "kept.txt\n").unwrap();
        fs::write(loose.join(".ignore"), "hidden.txt\n").unwrap();
        fs::write(loose.join("kept.txt"), "").unwrap();
        fs::write(loose.join("hidden.txt"), "").unwrap();

        let (matches, _) = find_names(&root, ".txt", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&matches), ["loose/kept.txt"]);
    }

    #[test]
    fn nothing_under_git_is_ever_found() {
        let root = scratch();
        let repo = checkout(&root, "app");
        fs::write(repo.join("HEAD.md"), "").unwrap();

        let (heads, _) = find_names(&root, "head", 500);
        let (configs, _) = find_names(&root, "config", 500);
        let (gits, _) = find_names(&root, "git", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&heads), ["app/HEAD.md"]);
        assert!(configs.is_empty(), "found {configs:?}");
        assert!(gits.is_empty(), "found {gits:?}");
    }

    #[test]
    fn stops_at_the_limit_and_says_it_was_cut_short() {
        let root = scratch();
        for n in 0..5 {
            fs::write(root.join(format!("page{n}.html")), "").unwrap();
        }

        let (limited, cut_short) = find_names(&root, "page", 3);
        let (exact, exact_cut_short) = find_names(&root, "page", 5);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            paths_of(&limited),
            ["page0.html", "page1.html", "page2.html"]
        );
        assert!(cut_short, "two more matched");
        assert_eq!(exact.len(), 5);
        assert!(!exact_cut_short, "exactly the limit is everything");
    }

    #[test]
    fn a_query_matching_nothing_finds_nothing() {
        let root = scratch();
        fs::write(checkout(&root, "app").join("main.rs"), "").unwrap();

        let (matches, cut_short) = find_names(&root, "docker-compose", 500);

        let _ = fs::remove_dir_all(&root);
        assert!(matches.is_empty());
        assert!(!cut_short);
    }

    #[test]
    fn an_empty_query_finds_nothing_rather_than_everything() {
        let root = scratch();
        fs::write(root.join("anything.txt"), "").unwrap();

        let (empty, _) = find_names(&root, "", 500);
        let (blank, _) = find_names(&root, "   ", 500);

        let _ = fs::remove_dir_all(&root);
        assert!(empty.is_empty());
        assert!(blank.is_empty());
    }

    /// A real, `rcgen`-generated self-signed EC certificate, subject
    /// `CN=Test Root CA, O=RepoSphereExplorer Test`, valid 1975-01-01 to
    /// 4096-01-01 - the same fixture `plugin-certificate`'s own tests use.
    const CERTIFICATE_PEM: &str = "-----BEGIN CERTIFICATE-----
MIIBkTCCATagAwIBAgIUf1zOrArsGiN2arJZNkQIT3HL6w4wCgYIKoZIzj0EAwIw
OTEVMBMGA1UEAwwMVGVzdCBSb290IENBMSAwHgYDVQQKDBdSZXBvU3BoZXJlRXhw
bG9yZXIgVGVzdDAgFw03NTAxMDEwMDAwMDBaGA80MDk2MDEwMTAwMDAwMFowOTEV
MBMGA1UEAwwMVGVzdCBSb290IENBMSAwHgYDVQQKDBdSZXBvU3BoZXJlRXhwbG9y
ZXIgVGVzdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABMMAKU4Arv7N+K5Xl/uo
GONeVXtrOhCcAUOf4StBpmlkgDo6hUfFTRj7IV5Txom86+qU5Jd6ADvPTzKeedWo
kuOjGjAYMBYGA1UdEQQPMA2CC2V4YW1wbGUuY29tMAoGCCqGSM49BAMCA0kAMEYC
IQCLgSlLPiOqHmY6oBKfdbCFqLHqgoZPgOGIdxzkiio+4AIhAJUvavI81fz1qqiW
Q8c1CP8QZQZVYgnOSYqC2s/Wyr6i
-----END CERTIFICATE-----
";

    /// A real, `rsa`-crate-generated 512-bit PKCS#8 private key - the same
    /// fixture `plugin-certificate`'s own tests use.
    const PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIBVQIBADANBgkqhkiG9w0BAQEFAASCAT8wggE7AgEAAkEA3RNJt9hafRyQ7kep
vIo+NOPMCH06/hDiNlSx9U5B8qzmUpy8O6JDeUaL6Zmuc0MYs3jGKqjlRS4jUbJv
s28Y8QIDAQABAkACPyHuplo1D0dBxKSq79S2AOKf63XgAxfpaW7tiUAOUUKn3O/N
UZgxrOOCKZKNAARiqZTZqqq8L6TZt3eVcFgBAiEA8t19Xc77fVrRqnp3SHj/hWve
RwuHl1pU8eY16gEddUECIQDpCB0w+AfTd/x/ZX5gYEq6/pVNcM4fdp5eQRUB44xH
sQIhANm20HHN4QkI5zfKPTBMt9NlVYeewFhf9BI960rw4PWBAiEAyGEjyNHe2OZa
Bqoda34hhH4ZoEeZ1tBHCcFo8QDbxWECIDNwkmvUEJEkNDKc+kxZj3fVUXbUhbze
Mo8hvqlfr/IR
-----END PRIVATE KEY-----
";

    #[test]
    fn finds_certificates_across_repositories_skips_git_and_reports_the_unreadable() {
        let root = scratch();
        let alpha = checkout(&root, "alpha");
        fs::write(alpha.join("server.pem"), CERTIFICATE_PEM).unwrap();
        fs::write(alpha.join(".git").join("hook.pem"), CERTIFICATE_PEM).unwrap();
        let beta = checkout(&root, "beta");
        fs::write(beta.join("server-key.pem"), PRIVATE_KEY_PEM).unwrap();
        fs::write(beta.join("broken.pem"), "not a pem file at all").unwrap();

        let findings = find_certificates(&root);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            paths_of_certificates(&findings),
            ["alpha/server.pem", "beta/broken.pem", "beta/server-key.pem"],
            "nothing under .git is ever found"
        );

        let certificate = &findings[0];
        assert_eq!(certificate.repository.as_deref(), Some("alpha"));
        match &certificate.kind {
            protocol::CertificateFindingKind::Blocks(blocks) => match &blocks[..] {
                [protocol::CertificateBlock::Certificate(summary)] => {
                    assert_eq!(
                        summary.subject,
                        "CN=Test Root CA, O=RepoSphereExplorer Test"
                    );
                    assert_eq!(summary.issuer, summary.subject);
                    assert!(summary.self_signed);
                    assert!(!summary.serial.is_empty());
                    assert!(summary.not_before < summary.not_after);
                }
                other => panic!("expected one certificate, got {other:?}"),
            },
            unreadable @ protocol::CertificateFindingKind::Unreadable => {
                panic!("expected blocks, got {unreadable:?}")
            }
        }

        let unreadable = &findings[1];
        assert_eq!(unreadable.repository.as_deref(), Some("beta"));
        assert_eq!(
            unreadable.kind,
            protocol::CertificateFindingKind::Unreadable
        );

        let key = &findings[2];
        assert_eq!(key.repository.as_deref(), Some("beta"));
        assert_eq!(
            key.kind,
            protocol::CertificateFindingKind::Blocks(vec![protocol::CertificateBlock::PrivateKey])
        );
    }

    /// The paths of [`find_certificates`]' findings, in order.
    fn paths_of_certificates(findings: &[protocol::CertificateFinding]) -> Vec<&str> {
        findings.iter().map(|found| found.path.as_str()).collect()
    }

    #[test]
    fn no_certificates_response_carries_a_private_keys_own_material() {
        let root = scratch();
        let alpha = checkout(&root, "alpha");
        fs::write(alpha.join("server-key.pem"), PRIVATE_KEY_PEM).unwrap();
        fs::write(alpha.join("server.pem"), CERTIFICATE_PEM).unwrap();

        let certificates = find_certificates(&root);
        let _ = fs::remove_dir_all(&root);

        let mut wire = Vec::new();
        protocol::write_message(
            &mut wire,
            &protocol::Response::Certificates {
                certificates,
                complete: true,
            },
        )
        .unwrap();
        let json = String::from_utf8(wire[4..].to_vec()).unwrap();

        // A snippet unique to the private key's own base64 body: if this
        // shows up on the wire, the key's bytes leaked with it.
        assert!(
            !json.contains("3RNJt9hafRyQ7kep"),
            "the response carries the private key's own material: {json}"
        );
    }

    #[test]
    fn the_match_ignores_case() {
        let root = scratch();
        fs::write(root.join("AppSettings.JSON"), "").unwrap();

        let (matches, _) = find_names(&root, "appsettings.json", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&matches), ["AppSettings.JSON"]);
    }

    #[test]
    fn a_matching_folder_is_found_as_a_folder() {
        let root = scratch();
        fs::create_dir_all(checkout(&root, "app").join("docker-compose")).unwrap();

        let (matches, _) = find_names(&root, "compose", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&matches), ["app/docker-compose"]);
        assert!(matches[0].is_dir);
        assert_eq!(matches[0].repository.as_deref(), Some("app"));
    }

    #[test]
    fn a_match_outside_every_repository_names_none() {
        let root = scratch();
        let loose = root.join("scratch");
        fs::create_dir_all(&loose).unwrap();
        fs::write(loose.join("todo.txt"), "").unwrap();

        let (matches, _) = find_names(&root, "todo", 500);

        let _ = fs::remove_dir_all(&root);
        assert_eq!(paths_of(&matches), ["scratch/todo.txt"]);
        assert!(!matches[0].is_dir);
        assert_eq!(matches[0].repository, None);
    }

    #[test]
    fn a_folder_that_is_not_a_working_copy_has_no_working_tree_status() {
        let dir = std::env::temp_dir().join(format!("rse-no-working-tree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let answer = handle_request(&protocol::Request::WorkingTreeStatus {
            path: dir.display().to_string(),
        });

        assert_eq!(working_tree_status(&dir), None);
        assert_eq!(
            answer,
            protocol::Response::WorkingTree {
                path: dir.display().to_string(),
                status: None,
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A socket file left by a service that was killed is taken over; one a
    /// running service holds is not.
    #[cfg(unix)]
    #[test]
    fn a_stale_socket_file_is_reclaimed_but_a_live_one_is_not() {
        use interprocess::local_socket::{GenericFilePath, ToFsName};

        let path = std::env::temp_dir().join(format!("rse-stale-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        // A socket file with nobody listening: what a killed service leaves.
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists(), "the fixture is a leftover socket file");
        let name = || path.clone().to_fs_name::<GenericFilePath>().unwrap();
        assert_eq!(
            super::bind(name()).err().map(|err| err.kind()),
            Some(std::io::ErrorKind::AddrInUse),
            "a plain bind is refused, which is the defect"
        );

        let live = super::bind_reclaiming_stale(name()).expect("the stale file is taken over");

        let second = super::bind_reclaiming_stale(name());
        assert_eq!(
            second.err().map(|err| err.kind()),
            Some(std::io::ErrorKind::AddrInUse),
            "a socket a live service holds is left alone"
        );
        drop(live);
        let _ = std::fs::remove_file(&path);
    }
}
