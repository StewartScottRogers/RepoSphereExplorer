# RepoSphereExplorer

A Rust application, built and maintained as a **dark factory**: work enters as a
GitHub issue, machines do the rest, and a release comes out the other end. No
step of the pipeline assumes a human is watching.

Current state: a working skeleton — a `clap` CLI over a library crate, with the
full pipeline wired and green. The explorer itself is not implemented yet.

## Run it

```bash
cargo run -- explore acme/widgets
```

## The floor

| Station | File | Trigger |
| --- | --- | --- |
| Inspection | [`ci.yml`](.github/workflows/ci.yml) | every push and PR: fmt, clippy (`-D warnings`), tests, release build, `cargo audit` |
| Assembly | [`claude.yml`](.github/workflows/claude.yml) | issue labelled `work-order`, or `@claude` in a comment |
| Restocking | [`dependabot.yml`](.github/dependabot.yml) | weekly cargo + actions updates |
| Dispatch | [`auto-merge.yml`](.github/workflows/auto-merge.yml) | PRs from Dependabot or labelled `auto-merge` |
| Shipping | [`release.yml`](.github/workflows/release.yml) | tag `v*`: Linux/Windows/macOS binaries attached to a GitHub release |

A work order ([template](.github/ISSUE_TEMPLATE/work-order.yml)) must state an
observable outcome and the acceptance checks that prove it. That contract is
what lets the factory run unattended: the checks decide whether the work landed,
not a reviewer's opinion.

## Turning the lights off

One-time setup on GitHub, all of it required before the floor runs itself:

1. Add the repository secret `ANTHROPIC_API_KEY` (or use
   `/install-github-app` from Claude Code, which sets it up for you).
2. Protect `main`: require the `fmt / clippy / test` check, and allow
   auto-merge in repository settings.
3. Create the labels `work-order`, `defect`, and `auto-merge`.

Until step 1 is done, `claude.yml` is inert and the repo behaves as an ordinary
CI-checked Rust project.

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
