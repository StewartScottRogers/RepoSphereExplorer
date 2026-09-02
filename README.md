# RepoSphereExplorer

A Rust application, built and maintained as a **dark factory**: work enters as a
GitHub issue, machines do the rest, and a release comes out the other end. No
step of the pipeline assumes a human is watching.

Current state: a cargo workspace implementing build order steps 1-4 of
[GUIDANCE.md](GUIDANCE.md) — a three-pane terminal explorer (`tui`: folders,
contents, file preview) backed by a local file-serving process (`service`),
with a text-file preview plugin. The `explore` CLI
(`repo_sphere_explorer`) is an earlier placeholder, kept for its own tests
but superseded by the explorer below. No GUI yet (build order step 5).

## Run it

Download `tui` and `service` for your platform from the
[latest release](https://github.com/StewartScottRogers/RepoSphereExplorer/releases/latest).
Rename off the target-triple suffix (e.g. `service-x86_64-pc-windows-msvc.exe`
to `service.exe`) so both binaries sit side by side, then run:

```bash
./tui [path]   # defaults to the current directory
```

`tui` starts `service` automatically if it isn't already running, and the
service keeps running afterwards so later launches reconnect instantly.
Only text files preview today; other types report that no plugin recognises
them. Binaries are unsigned (see [GUIDANCE.md §7 D3](GUIDANCE.md#7-open-decisions)),
so Windows SmartScreen and macOS Gatekeeper will warn on first run.

## The floor

| Station | File | Trigger |
| --- | --- | --- |
| Inspection | [`ci.yml`](.github/workflows/ci.yml) | every push and PR: fmt, clippy (`-D warnings`), tests, release build, `cargo audit` |
| Assembly | [`claude.yml`](.github/workflows/claude.yml) | issue labelled `work-order`, or `@claude` in a comment |
| Restocking | [`dependabot.yml`](.github/dependabot.yml) | weekly cargo + actions updates |
| Dispatch | [`auto-merge.yml`](.github/workflows/auto-merge.yml) | PRs from Dependabot or labelled `auto-merge` |
| Night shift | [`factory-shift.yml`](.github/workflows/factory-shift.yml) | nightly 03:00 UTC: builds the oldest open work order and opens a PR |
| Repair | [`repair.yml`](.github/workflows/repair.yml) | CI failed on `main`: diagnoses the run and opens a fix PR |
| Shipping | [`release.yml`](.github/workflows/release.yml) | tag `v*`: Linux/Windows/macOS binaries attached to a GitHub release |

A work order ([template](.github/ISSUE_TEMPLATE/work-order.yml)) must state an
observable outcome and the acceptance checks that prove it. That contract is
what lets the factory run unattended: the checks decide whether the work landed,
not a reviewer's opinion.

## Turning the lights off

One-time setup on GitHub, all of it required before the floor runs itself:

1. Run `/install-github-app` from Claude Code in this repository. It installs
   the Claude GitHub App and sets `ANTHROPIC_API_KEY`. Prefer this over setting
   the secret by hand: pull requests opened with the default `GITHUB_TOKEN` do
   **not** trigger workflows, so a hand-configured factory opens PRs that never
   run CI and therefore never satisfy auto-merge.
2. In Settings, Actions, General: allow GitHub Actions to create and approve
   pull requests.

Already configured on this repository: `main` requires the
`fmt / clippy / test` check with no human review, auto-merge is on, head
branches are deleted after merge, and the `work-order`, `defect` and
`auto-merge` labels exist.

Until step 1 is done the Claude stations are inert and the repo behaves as an
ordinary CI-checked Rust project.

## Local commands

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

Toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) (1.98,
edition 2024), so local and CI builds match.

## License

MIT — see [LICENSE](LICENSE).
