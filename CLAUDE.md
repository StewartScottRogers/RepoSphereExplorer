# RepoSphereExplorer

A Rust CLI (`repo_sphere_explorer`) developed as a dark factory: work orders in
as GitHub issues, releases out, no human on the floor.

## Commands

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-features
cargo run -- explore acme/widgets
```

All three checks must pass before a PR is opened. CI runs the same three plus
`cargo build --release` and `cargo audit`, so a green local run means a green
pipeline.

## Layout

A cargo workspace, per [GUIDANCE.md §5](GUIDANCE.md#5-proposed-workspace-layout):

- `crates/cli/` — today's `explore` command: `src/lib.rs` (all behaviour,
  testable without a process), `src/main.rs` (argument parsing only, a thin
  shell over the lib), `tests/cli.rs` (end-to-end tests against the binary).
- `crates/protocol/` — IPC message types shared by the service and both front
  ends.
- `crates/plugin-api/` — the core/presentation plugin traits and registration
  macro.
- `crates/service/` — the fat process: filesystem, index, operations, plugin
  cores.
- `crates/tui/`, `crates/gui/` — the Ratatui and Slint front ends.
- `crates/plugins/{text,image,archive,pdf,directory}/` — one crate per file
  type, each with a core half and a presentation half.

Shared lints and the release profile live once in the workspace root
`Cargo.toml`; member crates opt in with `[lints] workspace = true`.

## Design source

[GUIDANCE.md](GUIDANCE.md) is the design this project is built from. Every work
order should trace back to a line in it. If the guidance and the code disagree,
change the guidance first. Decisions D1-D5 in its section 7 gate the large work.

## Rules of the floor

1. **Acceptance checks are the definition of done.** A work order states its
   checks; the change is finished when they pass, not when the code looks right.
2. **New behaviour lands with a test.** Unit test in `lib.rs` for logic,
   integration test in `tests/cli.rs` for anything visible at the CLI.
3. **Lints are not negotiable.** `unsafe_code` is forbidden, `clippy::all` is
   deny, `missing_docs` warns. Fix the cause; do not add `#[allow]` without
   saying why in the same commit.
4. **Surgical changes.** Every changed line traces to the work order. Do not
   refactor adjacent code, reformat untouched files, or add dependencies that
   the work order did not call for.
5. **No speculative structure.** No abstraction for a single caller, no
   configurability nobody asked for, no error handling for impossible states.
6. **Stop and ask in the issue** when the work order is ambiguous, rather than
   guessing. An unattended wrong build costs more than a blocked one.
