//! The macOS application bundle: the shape of `Repos Explorer.app`, and the
//! property list (plist) that tells macOS what it is.
//!
//! A bundle is a folder with a shape, so laying one out is ordinary file
//! work. It lives here rather than in the release workflow because a shell
//! step on a runner is a thing no test can hold, and because
//! `scripts/install.sh` has to produce the very same folder for somebody who
//! installs from the command line - one layout on a Mac, not two. The plist
//! this crate writes is the plist that script writes, and
//! `the_install_script_writes_the_same_plist` is what keeps the two from
//! drifting apart.
//!
//! Nothing links this crate into a shipped binary: `release.yml` runs it on
//! the macOS runner to build the bundle the disk image carries.

use std::fs;
use std::path::Path;

/// The bundle's folder name. A macOS application is named by its folder, so
/// this is the name Finder, the Dock and Launchpad show.
pub const BUNDLE: &str = "Repos Explorer.app";

/// The reverse domain name macOS files this application's preferences,
/// window state and launch services registration under. `stewartscottrogers`
/// on `github.io` is where the release manifest is published from, so it is a
/// name this project actually holds.
pub const IDENTIFIER: &str = "io.github.stewartscottrogers.RepoSphereExplorer";

/// What the application calls itself, in the menu bar and under its icon.
pub const NAME: &str = "Repos Explorer";

/// The one of the three executables macOS starts when the bundle is opened.
pub const EXECUTABLE: &str = "RepoSphereExplorerGui";

/// The icon file inside `Contents/Resources`, written by `cargo run -p icon`
/// as `assets/RepoSphereExplorer.icns`.
pub const ICON: &str = "AppIcon.icns";

/// The oldest macOS this is built for. The release targets Apple silicon,
/// which no earlier system runs on.
pub const MINIMUM_SYSTEM: &str = "11.0";

/// Everything that goes into `Contents/MacOS`. The graphical application
/// starts the service from its own folder and the terminal application is a
/// second front end onto the same install, so all three are in the bundle
/// and a Mac has one copy of each.
pub const EXECUTABLES: [&str; 3] = ["RepoSphereExplorerGui", "RepoSphereExplorerTui", "service"];

/// The `Contents/Info.plist` of a bundle publishing release `version`.
///
/// `CFBundleVersion` and `CFBundleShortVersionString` are the same string:
/// the release version is the only version this project has, and macOS reads
/// the first as the build and the second as the one it shows a reader.
#[must_use]
pub fn info_plist(version: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleIdentifier</key>
    <string>{IDENTIFIER}</string>
    <key>CFBundleName</key>
    <string>{NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>{NAME}</string>
    <key>CFBundleExecutable</key>
    <string>{EXECUTABLE}</string>
    <key>CFBundleIconFile</key>
    <string>{ICON}</string>
    <key>CFBundleVersion</key>
    <string>{version}</string>
    <key>CFBundleShortVersionString</key>
    <string>{version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>{MINIMUM_SYSTEM}</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#
    )
}

/// Builds `Repos Explorer.app` at `into`: the property list, the icon, and
/// the three executables from `binaries`.
///
/// `into` is the bundle folder itself, not the folder holding it, so a
/// caller chooses where it lands and what it is called. An existing bundle
/// is written over rather than removed: the executables and the plist are
/// every file the bundle holds, so replacing them replaces all of it.
///
/// # Errors
///
/// When a folder cannot be made, or `binaries` is missing one of
/// [`EXECUTABLES`], or the icon cannot be copied.
pub fn lay_out(into: &Path, binaries: &Path, icon: &Path, version: &str) -> Result<(), String> {
    let contents = into.join("Contents");
    let executables = contents.join("MacOS");
    let resources = contents.join("Resources");
    for folder in [&executables, &resources] {
        fs::create_dir_all(folder)
            .map_err(|err| format!("could not make {}: {err}", folder.display()))?;
    }

    let plist = contents.join("Info.plist");
    fs::write(&plist, info_plist(version))
        .map_err(|err| format!("could not write {}: {err}", plist.display()))?;

    let placed_icon = resources.join(ICON);
    fs::copy(icon, &placed_icon).map_err(|err| {
        format!(
            "could not copy {} to {}: {err}",
            icon.display(),
            placed_icon.display()
        )
    })?;

    for name in EXECUTABLES {
        let from = binaries.join(name);
        let to = executables.join(name);
        fs::copy(&from, &to).map_err(|err| {
            format!(
                "could not copy {} to {}: {err}",
                from.display(),
                to.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        BUNDLE, EXECUTABLE, EXECUTABLES, ICON, IDENTIFIER, MINIMUM_SYSTEM, NAME, info_plist,
        lay_out,
    };
    use std::fs;
    use std::path::{Path, PathBuf};

    /// The plist's `<key>` and the element after it, in order.
    fn keys(plist: &str) -> Vec<(String, String)> {
        // A property list opens with Apple's document type declaration, which
        // the parser will not read past unless it is asked to.
        let options = roxmltree::ParsingOptions {
            allow_dtd: true,
            ..roxmltree::ParsingOptions::default()
        };
        let document = roxmltree::Document::parse_with_options(plist, options)
            .expect("the plist is well-formed XML");
        let dict = document
            .descendants()
            .find(|node| node.has_tag_name("dict"))
            .expect("the plist holds a dictionary");
        let mut pairs = Vec::new();
        let mut key: Option<String> = None;
        for node in dict.children().filter(roxmltree::Node::is_element) {
            if node.has_tag_name("key") {
                key = Some(node.text().unwrap_or_default().to_owned());
            } else if let Some(name) = key.take() {
                let value = match node.tag_name().name() {
                    "true" => "true".to_owned(),
                    "false" => "false".to_owned(),
                    _ => node.text().unwrap_or_default().to_owned(),
                };
                pairs.push((name, value));
            }
        }
        pairs
    }

    /// A fresh empty folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("rse-macos-bundle-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("a scratch folder");
        folder
    }

    /// The repository root, from this crate's own folder.
    fn repository() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("the repository is where this crate is")
    }

    /// Every key the work order asks for, and the three that make the folder
    /// an application rather than a folder of files.
    #[test]
    fn the_plist_says_what_macos_needs_to_know() {
        let found = keys(&info_plist("1.2.3"));
        assert_eq!(
            found,
            vec![
                ("CFBundleInfoDictionaryVersion".to_owned(), "6.0".to_owned()),
                ("CFBundlePackageType".to_owned(), "APPL".to_owned()),
                ("CFBundleIdentifier".to_owned(), IDENTIFIER.to_owned()),
                ("CFBundleName".to_owned(), NAME.to_owned()),
                ("CFBundleDisplayName".to_owned(), NAME.to_owned()),
                ("CFBundleExecutable".to_owned(), EXECUTABLE.to_owned()),
                ("CFBundleIconFile".to_owned(), ICON.to_owned()),
                ("CFBundleVersion".to_owned(), "1.2.3".to_owned()),
                ("CFBundleShortVersionString".to_owned(), "1.2.3".to_owned()),
                (
                    "LSMinimumSystemVersion".to_owned(),
                    MINIMUM_SYSTEM.to_owned()
                ),
                ("NSHighResolutionCapable".to_owned(), "true".to_owned()),
            ]
        );
    }

    /// `CFBundleName` is what macOS puts in the menu bar, and it truncates
    /// past fifteen characters.
    #[test]
    fn the_bundle_name_is_short_enough_for_the_menu_bar() {
        assert!(
            NAME.chars().count() <= 15,
            "{NAME} is longer than the menu bar shows"
        );
    }

    #[test]
    fn a_bundle_holds_the_plist_the_icon_and_the_three_executables() {
        let folder = scratch("laid-out");
        let binaries = folder.join("binaries");
        fs::create_dir_all(&binaries).expect("a folder of binaries");
        for name in EXECUTABLES {
            fs::write(binaries.join(name), format!("bytes of {name}")).expect("a binary");
        }
        let icon = folder.join("RepoSphereExplorer.icns");
        fs::write(&icon, b"icns").expect("an icon");

        let app = folder.join(BUNDLE);
        lay_out(&app, &binaries, &icon, "9.9.9").expect("the bundle is laid out");

        assert_eq!(
            fs::read_to_string(app.join("Contents/Info.plist")).expect("the plist is written"),
            info_plist("9.9.9")
        );
        assert_eq!(
            fs::read(app.join("Contents/Resources").join(ICON)).expect("the icon is placed"),
            b"icns"
        );
        for name in EXECUTABLES {
            assert_eq!(
                fs::read_to_string(app.join("Contents/MacOS").join(name))
                    .unwrap_or_else(|err| panic!("{name} is in Contents/MacOS: {err}")),
                format!("bytes of {name}")
            );
        }
        let _ = fs::remove_dir_all(&folder);
    }

    /// A release whose build dropped one of the three should fail loudly
    /// rather than ship a bundle that opens onto nothing.
    #[test]
    fn a_missing_executable_is_named() {
        let folder = scratch("missing");
        let binaries = folder.join("binaries");
        fs::create_dir_all(&binaries).expect("a folder of binaries");
        for name in EXECUTABLES.into_iter().filter(|name| *name != "service") {
            fs::write(binaries.join(name), b"bytes").expect("a binary");
        }
        let icon = folder.join("RepoSphereExplorer.icns");
        fs::write(&icon, b"icns").expect("an icon");

        let failure = lay_out(&folder.join(BUNDLE), &binaries, &icon, "9.9.9")
            .expect_err("a bundle missing the service is not a bundle");
        assert!(
            failure.contains("service"),
            "the failure should name what is missing: {failure}"
        );
        let _ = fs::remove_dir_all(&folder);
    }

    /// The seam. `scripts/install.sh` is downloaded and run on its own, so it
    /// cannot read this crate; it writes the property list itself, from a
    /// here-document. Two copies of the same text drift, and the one that
    /// would drift unnoticed is the script's - nothing in this workspace
    /// builds it. This reads that here-document out of the script, puts the
    /// version where the shell would, and holds it against what
    /// [`info_plist`] produces.
    #[test]
    fn the_install_script_writes_the_same_plist() {
        let script = repository().join("scripts/install.sh");
        let text = fs::read_to_string(&script)
            .unwrap_or_else(|err| panic!("{} should read: {err}", script.display()));
        let opened = text
            .split_once("<<INFO_PLIST\n")
            .unwrap_or_else(|| {
                panic!(
                    "{} should write the plist from an INFO_PLIST here-document",
                    script.display()
                )
            })
            .1;
        let body = opened
            .split_once("\nINFO_PLIST\n")
            .unwrap_or_else(|| {
                panic!(
                    "{}'s INFO_PLIST here-document should be closed",
                    script.display()
                )
            })
            .0;
        // An unquoted here-document delimiter is what lets the shell put the
        // release version in; nothing else in a property list is a dollar.
        let written = format!("{}\n", body.replace("$version", "4.5.6"));
        assert_eq!(
            written,
            info_plist("4.5.6"),
            "scripts/install.sh writes a different Info.plist from the one \
             release.yml puts in the disk image"
        );
    }
}
