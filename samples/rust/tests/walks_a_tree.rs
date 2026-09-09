//! The crate at its front door, as a caller meets it.

use std::fs;
use tree_indexer::index;

#[test]
fn finds_every_file_under_the_root() {
    let root = tempfile::tempdir().expect("a temporary directory");
    fs::create_dir(root.path().join("nested")).unwrap();
    fs::write(root.path().join("top.txt"), b"one").unwrap();
    fs::write(root.path().join("nested/deep.txt"), b"two").unwrap();

    let entries = index(root.path()).expect("the root is readable");

    let mut names: Vec<String> = entries
        .iter()
        .map(|entry| entry.relative.display().to_string().replace('\', "/"))
        .collect();
    names.sort();

    assert!(
        names.contains(&"nested/deep.txt".to_owned()),
        "the walk should descend: {names:?}"
    );
    assert!(names.contains(&"top.txt".to_owned()), "{names:?}");
}

#[test]
fn reports_where_it_stopped_when_a_root_cannot_be_read() {
    let missing = std::path::Path::new("no-such-directory-anywhere");

    let err = index(missing).expect_err("an unreadable root is an error");

    assert!(
        matches!(err, tree_indexer::Error::Halted { .. }),
        "and it says how far it got: {err}"
    );
}
