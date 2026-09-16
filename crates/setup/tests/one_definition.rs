//! One definition of what an install is, in two places that must not drift.
//!
//! `scripts/install.ps1` carries a rendered copy of `setup::layout`. This
//! fails when the committed script no longer matches what the module
//! renders, and rewrites it when run with `WRITE_INSTALL_SCRIPT=1`, so the
//! answer to a drift is always "regenerate", never "edit both".

use setup::layout;
use std::fs;
use std::path::PathBuf;

fn script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/install.ps1")
        .canonicalize()
        .expect("scripts/install.ps1 should be where the workspace keeps it")
}

/// Line endings are the checkout's business: this repository stores them as
/// line feeds and restores them on a Windows checkout.
fn lines_only(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[test]
fn the_script_carries_this_layout() {
    let path = script_path();
    let script = lines_only(&fs::read_to_string(&path).expect("the script should be readable"));
    let rendered = lines_only(&layout::powershell());

    let begin = script
        .find(layout::GENERATED_BEGIN)
        .expect("the script should carry the generated block's first line");
    let end = script
        .find(layout::GENERATED_END)
        .expect("the script should carry the generated block's last line")
        + layout::GENERATED_END.len();
    let carried = &script[begin..end];

    if carried == rendered {
        return;
    }
    if std::env::var_os("WRITE_INSTALL_SCRIPT").is_some() {
        let rewritten = format!("{}{rendered}{}", &script[..begin], &script[end..]);
        fs::write(&path, rewritten).expect("the script should be writable");
        return;
    }
    panic!(
        "{} has drifted from setup::layout. Run `WRITE_INSTALL_SCRIPT=1 cargo test -p setup` \
         to rewrite the generated block.\n\n--- the script carries ---\n{carried}\n\
         \n--- the layout renders ---\n{rendered}",
        path.display()
    );
}

#[test]
fn the_block_names_every_place_an_install_touches() {
    let rendered = layout::powershell();
    for expected in [
        layout::RECEIPT,
        layout::APPLICATION,
        layout::PUBLISHER,
        layout::SUMMARY,
        layout::REGISTRY_LINE,
        layout::REGISTRY_KEY,
        layout::PREFIX_UNDER_LOCAL_APPLICATION_DATA,
        layout::START_MENU_UNDER_APPLICATION_DATA,
    ] {
        assert!(
            rendered.contains(expected),
            "the block should name {expected}"
        );
    }
    for binary in layout::INSTALLED {
        assert!(rendered.contains(binary), "the block should name {binary}");
    }
}

#[test]
fn the_script_reads_its_places_from_the_block_rather_than_its_own_defaults() {
    let script = lines_only(&fs::read_to_string(script_path()).expect("the script is readable"));
    let opened = script
        .find("param(")
        .expect("the script should have a parameter block");
    let closed = script[opened..]
        .find("\n)")
        .expect("the parameter block should be closed")
        + opened;
    let parameters = &script[opened..closed];
    for spelled_out in [
        layout::PREFIX_UNDER_LOCAL_APPLICATION_DATA,
        layout::START_MENU_UNDER_APPLICATION_DATA,
        layout::REGISTRY_KEY,
    ] {
        assert!(
            !parameters.contains(spelled_out),
            "the parameter block still carries its own copy of {spelled_out}; the generated \
             block is the only place it should appear"
        );
    }
}
