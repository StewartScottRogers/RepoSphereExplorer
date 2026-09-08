# Repos Explorer

A front door to your development workspace: one window onto the local working
directories that source control systems — GitHub, GitLab, Bitbucket, Azure
DevOps, plain Git remotes — check code out into. It opens at your **Repos
Directory**, tells you which folders in it are clones and where each one came
from, and previews what is inside them.

It is deliberately **not** a general-purpose file explorer. Your operating
system already has one of those.

Built and maintained as a **dark factory**: work enters as a GitHub issue,
machines do the rest, and a release comes out the other end. No step of the
pipeline assumes a human is watching.

## What it does today

- **Opens where your code lives.** One configured Repos Directory — `Z:\repos`
  on Windows, `~/repos` elsewhere by default — and every launch starts there,
  wherever you were when you last closed it. Set it on first run, change it
  from the File menu.
- **Knows a clone when it sees one.** A folder holding a `.git` marker is
  marked as a repository and named with the provider it came from; a folder
  that is not one stays visible and looks different.
- **Reads the repository.** Select one and the File pane reports its provider,
  the branch checked out, and the remote it tracks.
- **Previews what is inside.** Eighty file types, each a plugin: source in
  every language the workspace holds, images, archives, documents, databases,
  fonts, media. A type may offer more than one view — its own rendering, and
  the file's plain text — and text types can be edited in place.
- **Handles files the way you expect.** Rename, copy, move, delete, extract an
  archive, create, undo the last one; multi-select, marquee, context menus,
  sortable columns, an address bar with history, drag-and-drop between panes.
  Every operation is confirmed before it runs and journaled afterwards.

Two front ends: a terminal user interface (TUI, Ratatui) and a graphical user
interface (GUI, Slint), both thin, over one service process that owns the
filesystem. Windows, macOS and Linux.

## What changed in the pivot

On 2026-09-08 this stopped being a general-purpose file explorer and became a
Repos Explorer. The reasoning is in [DECISIONS.md](DECISIONS.md) (D7 to D10);
the design is in [GUIDANCE.md](GUIDANCE.md).

| | Status |
| --- | --- |
| Three-pane browsing, file operations, plugins, previews | **Kept** — pointed at the Repos Directory rather than at the machine |
| Drive roots and "browse anywhere" as the opening view | **Removed** — the folders tree is rooted at the Repos Directory |
| Last-location session restore | **Never built, and now ruled out** — every launch opens at the root (D7) |
| Mounting network file sharing protocols | **Out of scope** — Server Message Block (SMB), Network File System (NFS) and their kin are the operating system's job. A Repos Directory on a drive the system has already mapped, such as `Z:`, is an ordinary path and works |
| Source control operations: clone, fetch, pull, commit | **Out of scope this phase** (D10) — detection is built so they can sit behind it later |
| Working-tree status (is this clone dirty?) | **Parked** — it needs the same walk of the work tree the operations will need; the field exists and reports unknown |
| `explore` command line placeholder (`repo_sphere_explorer`) | **Parked** — an early stub, kept for its own tests, scheduled for removal or repurposing |

## Run it

**[stewartscottrogers.github.io/RepoSphereExplorer](https://stewartscottrogers.github.io/RepoSphereExplorer/)**
has download links detected for your operating system. Or take binaries from
the [latest release](https://github.com/StewartScottRogers/RepoSphereExplorer/releases/latest):
`RepoSphereExplorerTui` and `service` for the terminal application, or
`RepoSphereExplorerGui` and `service` for the native one. Rename off the
target-triple suffix (for example `service-x86_64-pc-windows-msvc.exe` to
`service.exe`) so the pair sits side by side, then run:

```bash
./RepoSphereExplorerGui          # opens at your Repos Directory
./RepoSphereExplorerGui [path]   # an explicit path wins, for a one-off look
```

The binaries still carry the old product name; see
[Deferred renames](#deferred-renames).

On first run, with nothing configured, the application offers your platform's
default Repos Directory and takes whatever you give it instead. The choice is
stored per machine, and the next launch does not ask.

Either front end starts `service` automatically if it is not already running,
and the service keeps running afterwards so later launches reconnect instantly.
Every operation is journaled to `%LOCALAPPDATA%/RepoSphereExplorer/journal.jsonl`
(or the platform equivalent) with its exact target set and outcome. Binaries are
unsigned (see [GUIDANCE.md D3](GUIDANCE.md#7-decisions)), so Windows SmartScreen
and macOS Gatekeeper warn on first run.

Every binary takes a `--self-update` flag that checks the signed manifest
published alongside the Pages site and, if newer, downloads, verifies, and
replaces itself in place.

## Deferred renames

The pivot renamed the product, the window title and the user-facing strings.
These are not renamed yet, each for a reason:

| Name | Why it stayed |
| --- | --- |
| The repository, `StewartScottRogers/RepoSphereExplorer` | Renaming it moves the Pages site, the release download addresses, and the update manifest that installed copies poll. Needs a redirect plan first |
| Release binaries `RepoSphereExplorerGui` / `RepoSphereExplorerTui` | Installed copies self-update by name against the published manifest; renaming breaks the upgrade path for anyone already running one |
| Settings and journal directory `RepoSphereExplorer/` | Holds live user data. A rename needs a migration that moves the existing directory rather than orphaning it |
| Cargo package `repo_sphere_explorer` (the `explore` placeholder) | Parked rather than renamed, since it is scheduled for removal |

## Watch it build

**[Repository history film](https://stewartscottrogers.github.io/RepoSphereExplorer/#film)**
— a [Gource](https://gource.io/) visualization of every commit, re-rendered
nightly at 06:00 Coordinated Universal Time (UTC) (see
[`pages.yml`](.github/workflows/pages.yml)) so it stays current with whatever
the factory merged since yesterday, not just what shipped in the last release.
Direct link to the current video:
[`gource.mp4`](https://stewartscottrogers.github.io/RepoSphereExplorer/gource.mp4).

## The floor

| Station | File | Trigger |
| --- | --- | --- |
| Inspection | [`ci.yml`](.github/workflows/ci.yml) | every push and pull request: `fmt`, `clippy` (`-D warnings`), tests, release build, `cargo audit` |
| Assembly | [`claude.yml`](.github/workflows/claude.yml) | issue labelled `work-order`, or `@claude` in a comment |
| Restocking | [`dependabot.yml`](.github/dependabot.yml) | weekly cargo and actions updates |
| Dispatch | [`auto-merge.yml`](.github/workflows/auto-merge.yml) | pull requests from Dependabot or labelled `auto-merge` |
| Night shift | [`factory-shift.yml`](.github/workflows/factory-shift.yml) | nightly 03:00 UTC, or dispatched directly: builds the oldest open work order and opens a pull request |
| Day shift | [`day-shift.yml`](.github/workflows/day-shift.yml) | a Night shift run finishes cleanly: re-dispatches the next one while open work orders remain |
| Quality control | [`floor-health-check.yml`](.github/workflows/floor-health-check.yml) | every 30 minutes: unsticks auto-merge pull requests whose check never ran (or closes them if stale), and closes work-order issues already resolved by a merged pull request |
| Closing | [`close-linked-issues.yml`](.github/workflows/close-linked-issues.yml) | a pull request closes: closes the issues its `Closes #N` named, in seconds rather than on the next sweep |
| Independent review | [`independent-review.yml`](.github/workflows/independent-review.yml) | a work-order pull request opens or updates: a second, separately-invoked agent re-runs the checks and reviews the diff against CLAUDE.md — informational, not yet required |
| Repair | [`repair.yml`](.github/workflows/repair.yml) | checks failed on `main`: diagnoses the run and opens a fix pull request |
| Shipping | [`release.yml`](.github/workflows/release.yml) | tag `v*`: Linux, Windows and macOS binaries attached to a GitHub release |
| Storefront | [`pages.yml`](.github/workflows/pages.yml) | after `release.yml` finishes, or nightly at 06:00 UTC: signs the release, publishes `latest.json`, renders the history film, regenerates statistics, and deploys the Pages site |

A work order ([template](.github/ISSUE_TEMPLATE/work-order.yml)) must state an
observable outcome and the acceptance checks that prove it. That contract is
what lets the factory run unattended: the checks decide whether the work landed,
not a reviewer's opinion.

## Turning the lights off

One-time setup on GitHub, all of it required before the floor runs itself:

1. Run `/install-github-app` from Claude Code in this repository. It installs
   the Claude GitHub App and sets `ANTHROPIC_API_KEY`. Prefer this over setting
   the secret by hand: pull requests opened with the default `GITHUB_TOKEN` do
   **not** trigger workflows, so a hand-configured factory opens pull requests
   that never run their checks and therefore never satisfy auto-merge.
2. In Settings, Actions, General: allow GitHub Actions to create and approve
   pull requests.

Already configured on this repository: `main` requires the
`fmt / clippy / test` check with no human review, auto-merge is on, head
branches are deleted after merge, and the `work-order`, `defect` and
`auto-merge` labels exist.

Until step 1 is done the Claude stations are inert and the repository behaves as
an ordinary checked Rust project.

## Local commands

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --all-features
```

Toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) (1.98,
edition 2024), so local builds and the factory's match.

## License

MIT — see [LICENSE](LICENSE).
