//! #688 (CLAUDE.md rule 14): `no_terminal_attached_message` has a unit test
//! of its own, and `main` has always called it before anything else, yet a
//! release binary on Windows still switched to the alternate screen and
//! drew into a redirected file. The two halves individually did what they
//! claimed; nothing had driven `main` itself with stdout somewhere other
//! than a terminal to prove the guard actually stops the process.

use std::fs::{self, File};
use std::process::{Command, Stdio};

#[test]
fn refuses_to_draw_when_stdout_is_redirected() {
    let out_path =
        std::env::temp_dir().join(format!("rse-tui-no-terminal-out-{}", std::process::id()));
    let out_file = File::create(&out_path).expect("create the redirected stdout file");

    let output = Command::new(env!("CARGO_BIN_EXE_RepoSphereExplorerTui"))
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::piped())
        .output()
        .expect("run the terminal front end with stdout redirected to a file");

    let drawn = fs::read(&out_path).expect("read back the redirected stdout file");
    fs::remove_file(&out_path).ok();

    assert!(
        !output.status.success(),
        "a redirected stdout must exit with a failure, not run the event loop"
    );
    assert!(
        drawn.is_empty(),
        "the redirected file must stay empty, not receive escape sequences"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no terminal attached"),
        "the refusal message must reach stderr, got: {stderr:?}"
    );
}
