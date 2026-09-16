//! The desktop entry the Linux install writes, and the name the window
//! gives itself.
//!
//! Two halves again (CLAUDE.md rule 14). `scripts/install.sh` writes an
//! entry whose `StartupWMClass` is the only thing that gets a desktop
//! environment from an open window back to the entry, and so to the icon it
//! shows in the taskbar; `gui` sets the name the window carries. Either half
//! can be changed on its own and nothing breaks - the entry still validates,
//! the window still opens - and the reader gets a nameless window with a
//! blank icon and two taskbar buttons. A test of the entry alone, or of the
//! application id alone, would have watched that happen.
//!
//! `scripts/appimage.sh` writes the same entry into the `AppImage`, for the
//! reader who never runs the install script at all, so it has to say the
//! same things.
//!
//! What this cannot do is watch a desktop environment read either of them.
//! `distribution.yml` does that on the Linux runner: it validates the
//! installed entry with `desktop-file-validate` and reads `WM_CLASS` off
//! the open window with `xprop`.

use std::collections::BTreeMap;

/// The install script, which writes the entry a reader gets.
const INSTALL: &str = include_str!("../../../scripts/install.sh");

/// The `AppImage` script, which writes the entry inside the one-file build.
const APPIMAGE: &str = include_str!("../../../scripts/appimage.sh");

/// A stand-in for the installed graphical application, which the install
/// script's entry names by the path it put it at.
const INSTALLED_GUI: &str = "/home/reader/.local/share/RepoSphereExplorer/RepoSphereExplorerGui";

/// The body of the `DESKTOP` heredoc in `script`, with the shell expansions
/// the script would have made.
fn entry_in(script: &str) -> String {
    let opening = "<<DESKTOP\n";
    let start = script
        .find(opening)
        .expect("the script writes a desktop entry")
        + opening.len();
    let rest = &script[start..];
    let end = rest.find("\nDESKTOP\n").expect("the heredoc is closed");
    rest[..end]
        .replace("$ICON_NAME", icon::THEMED_NAME)
        .replace("\"$1\"", &format!("\"{INSTALLED_GUI}\""))
}

/// The entry's keys and values, the group header dropped.
fn keys(entry: &str) -> BTreeMap<&str, &str> {
    let mut lines = entry.lines();
    assert_eq!(
        lines.next(),
        Some("[Desktop Entry]"),
        "an entry opens with its group header or nothing reads it"
    );
    let mut keys = BTreeMap::new();
    for line in lines {
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("`{line}` is not a key and a value"));
        assert!(!value.is_empty(), "{key} has no value");
        assert!(
            keys.insert(key, value).is_none(),
            "{key} is given twice, and a reader takes whichever it likes"
        );
    }
    keys
}

#[test]
fn the_name_the_window_carries_is_the_name_the_entry_matches() {
    i_slint_backend_testing::init_no_event_loop();
    let ui = gui::MainWindow::new().expect("the window should build");
    // The same call `main` makes, in the same place: after the window
    // exists and before it is shown.
    gui::name_the_window().expect("the window takes the application id");
    drop(ui);

    let entry = entry_in(INSTALL);
    assert_eq!(
        keys(&entry).get("StartupWMClass"),
        Some(&gui::XDG_APP_ID),
        "the entry matches a window this application never opens"
    );
}

#[test]
fn the_icon_the_entry_names_is_the_icon_the_install_writes() {
    assert_eq!(
        gui::XDG_APP_ID,
        icon::THEMED_NAME,
        "the window's name and the icon's name have parted, so the desktop \
         finds no picture for the window"
    );
    let entry = entry_in(INSTALL);
    assert_eq!(keys(&entry).get("Icon"), Some(&icon::THEMED_NAME));
    for fragment in [
        "$applications/$ICON_NAME.desktop",
        "$icon_directory/$ICON_NAME.png",
        "$icons/scalable/apps/$ICON_NAME.svg",
    ] {
        assert!(
            INSTALL.contains(fragment),
            "the install script no longer writes {fragment}"
        );
    }
}

/// The keys the freedesktop Desktop Entry Specification wants of an
/// application, and the ones this work order asked for by name.
#[test]
fn the_entry_says_what_a_desktop_environment_needs() {
    let entry = entry_in(INSTALL);
    let keys = keys(&entry);
    assert_eq!(keys.get("Type"), Some(&"Application"));
    assert_eq!(keys.get("Name"), Some(&"Repos Explorer"));
    assert_eq!(keys.get("Terminal"), Some(&"false"));
    assert_eq!(
        keys.get("Categories"),
        Some(&"Development;Utility;FileTools;"),
        "the categories decide which menu it appears in"
    );
    let comment = keys.get("Comment").expect("an entry carries a comment");
    assert!(
        comment.len() > 20 && !comment.contains("file explorer"),
        "the comment is what a menu shows under the name: {comment}"
    );
    let exec = keys.get("Exec").expect("an entry carries a command");
    assert!(
        exec.ends_with(" %f"),
        "a path dropped on the entry has to reach the application: {exec}"
    );
    assert!(
        exec.contains(INSTALLED_GUI),
        "the entry runs something other than the installed application: {exec}"
    );
}

#[test]
fn the_appimage_describes_the_same_application() {
    let installed = entry_in(INSTALL);
    let carried = entry_in(APPIMAGE);
    let installed = keys(&installed);
    let carried = keys(&carried);
    assert_eq!(
        installed.keys().collect::<Vec<_>>(),
        carried.keys().collect::<Vec<_>>(),
        "the two entries describe the application with different keys"
    );
    for (key, value) in &installed {
        if *key == "Exec" {
            // The AppImage is mounted somewhere different every run, so
            // its entry names the binary and lets AppRun's PATH find it.
            assert_eq!(
                carried.get(key),
                Some(&"RepoSphereExplorerGui %f"),
                "the AppImage's entry runs something else"
            );
            continue;
        }
        assert_eq!(
            carried.get(key),
            Some(value),
            "{key} differs between the install script and the AppImage"
        );
    }
}
