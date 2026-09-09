# Repos Explorer

A cross-platform Repos Explorer — a front door to the local working
directories that source control systems check code out into — developed as a
dark factory: work orders in as GitHub issues, releases out, no human on the
floor.

It is not a general-purpose file explorer. Before building anything, ask
whether it helps somebody reach or understand the repositories they work in.
If the honest answer is "a file explorer would have this", that is a reason to
stop, not a reason to build. See [GUIDANCE.md §0](GUIDANCE.md#0-what-this-application-is-for)
and [DECISIONS.md](DECISIONS.md) D7 to D10.

## Commands

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -p gui                 # opens at the configured Repos Directory
```

All three checks must pass before a pull request is opened. The factory runs
the same three plus `cargo build --release` and `cargo audit`, so a green local
run means a green pipeline.

## Layout

A cargo workspace, per [GUIDANCE.md §5](GUIDANCE.md#5-proposed-workspace-layout):

- `crates/cli/` — the `explore` placeholder from before the pivot: `src/lib.rs`
  (all behaviour, testable without a process), `src/main.rs` (argument parsing
  only), `tests/cli.rs`. Parked; do not extend it.
- `crates/protocol/` — inter-process communication (IPC) message types shared
  by the service and both front ends.
- `crates/plugin-api/` — the core and presentation plugin traits.
- `crates/service/` — the fat process: filesystem, repositories, configuration,
  operations, plugin cores.
- `crates/tui/`, `crates/gui/` — the Ratatui terminal and Slint graphical front
  ends.
- `crates/plugins/*/` — one crate per file type, each with a core half and a
  presentation half, plus the folder plugins, which describe a folder rather
  than a file. See [PLUGINS.md](PLUGINS.md) for the built and rejected
  registry.
- `samples/` — one fixture directory per plugin. See
  [samples/README.md](samples/README.md).

Shared lints and the release profile live once in the workspace root
`Cargo.toml`; member crates opt in with `[lints] workspace = true`.

## Design source

[GUIDANCE.md](GUIDANCE.md) is the design this project is built from. Every work
order should trace back to a line in it. If the guidance and the code disagree,
change the guidance first. Decisions D1 to D10 in its section 7 gate the large
work; [DECISIONS.md](DECISIONS.md) records why the settled ones were settled
and what it would take to revisit them.

## Rules of the floor

1. **Acceptance checks are the definition of done.** A work order states its
   checks; the change is finished when they pass, not when the code looks right.
2. **New behaviour lands with a test.** Unit test in `lib.rs` for logic,
   integration test for anything visible at the surface.
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
7. **The Repos Directory is the anchor.** Every launch opens at the configured
   root; there is no last-location session restore, and none is to be added
   (D7). The root is stored as a list with one entry marked active (D9), and
   the boundary around it is soft — one configuration point, not a rule spread
   through the navigation code (D8).
8. **Detect, do not drive.** Repository awareness means reading: the `.git`
   marker, the provider in the remote address, the branch. Clone, fetch, pull
   and commit are out of scope (D10). Never run a source control command on a
   user's working copy.
9. **A folder is several things at once; a file is one.** A file plugin wins
   or loses. Every folder plugin that recognises a folder contributes, and its
   lines are added below what the folder already reports, never in place of
   them (D12). Sniff a folder only when it is the selected one — a full
   directory read per row would make a listing crawl (GUIDANCE.md §3.4).
10. **Track plugins in [PLUGINS.md](PLUGINS.md).** Before proposing or building
    a plugin, check its Built, Folder and Rejected sections, plus
    `gh issue list --label work-order --state all`, for that format. When a
    plugin's pull request merges, add it to the right section. When a work
    order concludes a format cannot reasonably become a plugin, add it to
    Rejected with a one-line reason and close the issue without a pull
    request — do not retry a rejected format unless explicitly asked to.
11. **Label a work order's pull request `auto-merge`.** Once the acceptance
    checks pass and the pull request is open, apply the `auto-merge` label so it
    lands itself under the same no-human-review policy that already governs
    `main` — do not wait for manual review, and do not merge it directly
    yourself.
12. **Keep `samples/` current.** A plugin work order that adds a new plugin
    crate must also add its `samples/<name>/` entry with a real, valid example
    file, in the same pull request. Make the fixture exercise every field the
    plugin extracts: `sample_coverage.rs` fails when a fixture stops proving
    anything. A programming language's directory holds a whole project — the
    manifest, the lock file, the source tree, a test — because a project is
    what a reader meets. The files are plausible, never toolchain-verified;
    see [samples/README.md](samples/README.md).
13. **Spell out an acronym before using it.** First use gives the full term,
    with the acronym in parentheses after it. This applies to documentation,
    commit messages, work orders and user-facing strings.
