//! Go project folder plugin: core and presentation halves.
//!
//! A Go checkout's identity is its module path, declared in `go.mod` and
//! rarely the same as the directory it sits in. This reads that
//! declaration and reports it, alongside what else the manifest says about
//! the module: the Go version and toolchain it asks for, how many direct
//! requirements it names, whether any is replaced by a sibling path or a
//! fork, and whether the checkout vendors its dependencies. A `cmd`
//! directory is where Go convention puts one subdirectory per command the
//! module builds, so its immediate subdirectories are counted too.
//!
//! Per decision D10 this reads and never drives: nothing here runs `go`,
//! and nothing here writes to the folder. `go.mod` states what the author
//! declared, which is the question a reader standing in the folder is
//! asking; resolving it into what would actually build needs the module
//! graph and the network, and is a different job from describing a folder.

use plugin_api::{FolderCore, FolderPresentation};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

/// The file that makes a folder a Go module.
const MANIFEST: &str = "go.mod";

/// The directory a vendored checkout keeps its dependencies in.
const VENDOR: &str = "vendor";

/// The directory Go convention holds one subdirectory per built command in.
const COMMANDS_DIR: &str = "cmd";

/// The largest manifest this will read. A `go.mod` is a handful of lines
/// per requirement; a file past this is not one, and a folder plugin runs
/// while somebody is waiting for a pane to draw.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// View data produced by [`GoProjectCore::view`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoProjectView {
    /// The module path the project publishes as.
    pub module: String,
    /// The Go language version the manifest asks for, when it declares one.
    pub go_version: Option<String>,
    /// The toolchain the manifest asks for, when it declares one.
    pub toolchain: Option<String>,
    /// How many `require` entries are direct - stated by the module
    /// itself rather than pulled in only to satisfy one of those.
    pub direct_requirements: usize,
    /// Whether any requirement is replaced by a sibling path or a fork.
    pub replaced: bool,
    /// Whether dependencies are vendored in the checkout.
    pub vendored: bool,
    /// How many commands the module builds: the immediate subdirectories
    /// of `cmd`, which is where Go convention puts one per binary.
    pub commands: usize,
}

/// The core half: recognises the folder and reads its module file.
pub struct GoProjectCore;

/// The presentation half: turns the module file into lines.
pub struct GoProjectPresentation;

/// What one line of `go.mod` declares, once any trailing `// indirect`
/// marker on a requirement has been noted and its comment removed.
struct Parsed {
    /// The module path.
    module: String,
    /// The `go` directive's value, when the manifest states one.
    go_version: Option<String>,
    /// The `toolchain` directive's value, when the manifest states one.
    toolchain: Option<String>,
    /// How many `require` entries were not marked `// indirect`.
    direct_requirements: usize,
    /// Whether a `replace` directive appeared anywhere.
    replaced: bool,
}

/// `line` with any trailing `//` comment removed.
fn without_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => line[..index].trim_end(),
        None => line,
    }
}

/// Reads a `go.mod`'s declarations from its text.
///
/// `go.mod` is not TOML or any other general format: it is line-oriented,
/// with `require` and `replace` able to take either a single line or a
/// parenthesised block of several. This reads exactly that shape, and
/// nothing past it - `exclude` and `retract` name versions rather than
/// requirements, and are not a fact this plugin reports.
fn parse(text: &str) -> Option<Parsed> {
    let mut module = None;
    let mut go_version = None;
    let mut toolchain = None;
    let mut direct_requirements = 0usize;
    let mut replaced = false;
    let mut in_require_block = false;
    let mut in_replace_block = false;

    for raw_line in text.lines() {
        let indirect = raw_line.trim_end().ends_with("// indirect");
        let line = without_comment(raw_line).trim();

        if in_require_block {
            if line == ")" {
                in_require_block = false;
            } else if !line.is_empty() && !indirect {
                direct_requirements += 1;
            }
            continue;
        }
        if in_replace_block {
            if line == ")" {
                in_replace_block = false;
            } else if !line.is_empty() {
                replaced = true;
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("module ") {
            module = Some(rest.trim().to_owned());
        } else if line == "require (" {
            in_require_block = true;
        } else if line.strip_prefix("require ").is_some() {
            if !indirect {
                direct_requirements += 1;
            }
        } else if line == "replace (" {
            in_replace_block = true;
        } else if line.starts_with("replace ") {
            replaced = true;
        } else if let Some(rest) = line.strip_prefix("toolchain ") {
            toolchain = Some(rest.trim().to_owned());
        } else if let Some(rest) = line.strip_prefix("go ") {
            go_version = Some(rest.trim().to_owned());
        }
    }

    module.map(|module| Parsed {
        module,
        go_version,
        toolchain,
        direct_requirements,
        replaced,
    })
}

/// How many commands the module builds: the immediate subdirectories of
/// `cmd`, which is where Go convention puts one per binary. Zero when
/// there is no `cmd` directory at all.
fn commands_in(cmd_dir: &Path) -> usize {
    std::fs::read_dir(cmd_dir).map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .count()
    })
}

impl FolderCore for GoProjectCore {
    fn name(&self) -> &'static str {
        "project-go"
    }

    fn sniff(&self, entries: &[&str]) -> bool {
        entries.contains(&MANIFEST)
    }

    fn view(&self, path: &Path) -> io::Result<serde_json::Value> {
        let manifest_path = path.join(MANIFEST);
        if std::fs::metadata(&manifest_path)?.len() > MAX_MANIFEST_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{MANIFEST} is larger than a manifest can reasonably be"),
            ));
        }
        let text = std::fs::read_to_string(&manifest_path)?;
        let parsed = parse(&text).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{MANIFEST} names no module"),
            )
        })?;

        let view = GoProjectView {
            module: parsed.module,
            go_version: parsed.go_version,
            toolchain: parsed.toolchain,
            direct_requirements: parsed.direct_requirements,
            replaced: parsed.replaced,
            vendored: path.join(VENDOR).is_dir(),
            commands: commands_in(&path.join(COMMANDS_DIR)),
        };
        serde_json::to_value(view).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

impl FolderPresentation for GoProjectPresentation {
    fn name(&self) -> &'static str {
        "project-go"
    }

    fn present(&self, data: &serde_json::Value) -> Vec<String> {
        let Ok(view) = serde_json::from_value::<GoProjectView>(data.clone()) else {
            return vec!["Go module: unreadable manifest".to_owned()];
        };

        let mut lines = vec![format!("Go module: {}", view.module)];

        lines.push(match (&view.go_version, &view.toolchain) {
            (Some(go_version), Some(toolchain)) => {
                format!("Go: {go_version} (toolchain {toolchain})")
            }
            (Some(go_version), None) => format!("Go: {go_version}"),
            (None, Some(toolchain)) => format!("Toolchain: {toolchain}"),
            (None, None) => "Go version: unstated".to_owned(),
        });

        lines.push(format!("Requirements: {} direct", view.direct_requirements));
        if view.replaced {
            lines.push(
                "Replaces at least one requirement with a sibling path or a fork.".to_owned(),
            );
        }
        if view.vendored {
            lines.push("Dependencies are vendored in the checkout.".to_owned());
        }
        lines.push(format!("Commands: {}", view.commands));

        lines
    }
}

#[cfg(test)]
mod tests {
    use super::{GoProjectCore, GoProjectPresentation, GoProjectView};
    use plugin_api::{FolderCore, FolderPresentation};
    use std::path::{Path, PathBuf};

    /// A folder under the temporary directory, holding `manifest` as its
    /// `go.mod`.
    fn folder_with(label: &str, manifest: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rse-project-go-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("go.mod"), manifest).unwrap();
        dir
    }

    fn view_of(dir: &Path) -> GoProjectView {
        let data = GoProjectCore.view(dir).unwrap();
        serde_json::from_value(data).unwrap()
    }

    #[test]
    fn a_folder_with_a_manifest_is_recognised() {
        assert!(GoProjectCore.sniff(&["cmd", "go.mod", "go.sum"]));
    }

    #[test]
    fn a_folder_without_one_is_not() {
        assert!(!GoProjectCore.sniff(&["src", "package.json"]));
        assert!(!GoProjectCore.sniff(&[]));
        // The checksum database is not the manifest: a folder can hold
        // one without being a module root, and Go itself goes by `go.mod`.
        assert!(!GoProjectCore.sniff(&["go.sum"]));
    }

    #[test]
    fn a_module_reports_its_path_version_and_toolchain() {
        let dir = folder_with(
            "basic",
            "module github.com/example/greeter\n\ngo 1.22\n\ntoolchain go1.22.3\n",
        );

        let view = view_of(&dir);

        assert_eq!(view.module, "github.com/example/greeter");
        assert_eq!(view.go_version.as_deref(), Some("1.22"));
        assert_eq!(view.toolchain.as_deref(), Some("go1.22.3"));
        assert_eq!(view.direct_requirements, 0);
        assert!(!view.replaced);
        assert!(!view.vendored);
        assert_eq!(view.commands, 0);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_require_block_counts_direct_requirements_and_skips_indirect_ones() {
        let dir = folder_with(
            "requirements",
            r"module github.com/example/greeter

go 1.22

require (
	github.com/spf13/cobra v1.8.1
	github.com/google/uuid v1.6.0
	golang.org/x/sync v0.7.0 // indirect
)
",
        );

        let view = view_of(&dir);

        assert_eq!(view.direct_requirements, 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_single_line_require_is_counted_the_same_as_a_block_entry() {
        let dir = folder_with(
            "single-line",
            "module github.com/example/tiny\n\ngo 1.22\n\nrequire github.com/pkg/errors v0.9.1\n",
        );

        let view = view_of(&dir);

        assert_eq!(view.direct_requirements, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_replace_directive_is_reported_whether_single_line_or_a_block() {
        let single = folder_with(
            "replace-single",
            "module github.com/example/greeter\n\ngo 1.22\n\nreplace github.com/spf13/cobra => github.com/example/cobra-fork v1.8.1-patched\n",
        );
        assert!(view_of(&single).replaced);
        std::fs::remove_dir_all(&single).unwrap();

        let block = folder_with(
            "replace-block",
            "module github.com/example/greeter\n\ngo 1.22\n\nreplace (\n\tgithub.com/spf13/cobra => github.com/example/cobra-fork v1.8.1-patched\n)\n",
        );
        assert!(view_of(&block).replaced);
        std::fs::remove_dir_all(&block).unwrap();

        let none = folder_with(
            "no-replace",
            "module github.com/example/greeter\n\ngo 1.22\n",
        );
        assert!(!view_of(&none).replaced);
        std::fs::remove_dir_all(&none).unwrap();
    }

    #[test]
    fn a_vendor_directory_is_reported_as_vendored() {
        let dir = folder_with("vendored", "module github.com/example/greeter\n\ngo 1.22\n");
        std::fs::create_dir_all(dir.join("vendor")).unwrap();

        assert!(view_of(&dir).vendored);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn commands_are_counted_from_the_cmd_directory() {
        let dir = folder_with("commands", "module github.com/example/greeter\n\ngo 1.22\n");
        std::fs::create_dir_all(dir.join("cmd").join("greeter")).unwrap();
        std::fs::create_dir_all(dir.join("cmd").join("worker")).unwrap();

        assert_eq!(view_of(&dir).commands, 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_manifest_with_no_module_directive_is_an_error_rather_than_a_guess() {
        let dir = folder_with("broken", "go 1.22\n");

        let refused = GoProjectCore.view(&dir);

        assert!(refused.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presents_a_module_as_lines_a_reader_can_use() {
        let data = GoProjectCore
            .view(&folder_with(
                "present",
                r"module github.com/example/greeter

go 1.22

toolchain go1.22.3

require (
	github.com/spf13/cobra v1.8.1
	golang.org/x/sync v0.7.0 // indirect
)

replace github.com/spf13/cobra => github.com/example/cobra-fork v1.8.1-patched
",
            ))
            .unwrap();

        let lines = GoProjectPresentation.present(&data);

        assert_eq!(
            lines,
            vec![
                "Go module: github.com/example/greeter",
                "Go: 1.22 (toolchain go1.22.3)",
                "Requirements: 1 direct",
                "Replaces at least one requirement with a sibling path or a fork.",
                "Commands: 0",
            ]
        );
    }

    #[test]
    fn both_halves_answer_to_the_same_name() {
        assert_eq!(
            FolderCore::name(&GoProjectCore),
            FolderPresentation::name(&GoProjectPresentation),
            "the service names the plugin that produced a view, and the front end looks the \
             presentation half up by that name"
        );
    }

    #[test]
    fn the_repository_fixture_reads_as_the_project_it_declares() {
        // `samples/project-go/` is a real module - a manifest naming a
        // module with no source would be a manifest for something that
        // cannot build, and this sample set holds working files.
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../samples/project-go");

        let data = GoProjectCore.view(&dir).unwrap();
        let view: GoProjectView = serde_json::from_value(data).unwrap();

        assert_eq!(view.module, "github.com/example/greeter");
        assert_eq!(view.go_version.as_deref(), Some("1.22"));
        assert_eq!(view.toolchain.as_deref(), Some("go1.22.3"));
        assert_eq!(view.direct_requirements, 2);
        assert!(view.replaced);
        assert!(!view.vendored);
        assert_eq!(view.commands, 1);
    }
}
