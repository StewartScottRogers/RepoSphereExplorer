# Repos Explorer

## Repository history

**[Repository history film](https://stewartscottrogers.github.io/RepoSphereExplorer/#film)**
— a [Gource](https://gource.io/) visualization of every commit, re-rendered
nightly at 06:00 Coordinated Universal Time (UTC) (see
[`pages.yml`](.github/workflows/pages.yml)) so it stays current with whatever
the factory merged since yesterday, not just what shipped in the last release.
Direct link to the current video:
[`gource.mp4`](https://stewartscottrogers.github.io/RepoSphereExplorer/gource.mp4).

## What it is

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
- **Knows a clone when it sees one.** A folder holding a `.git` marker carries
  a provider-tinted branch badge in both panes, its name in the provider's
  accent with its branch beside it, `Git repository · github.com` in Type,
  and an `M` for uncommitted changes; a folder that is not one stays visible
  and looks different.
- **Reads the repository.** Select one and the tool slot shows a table of its
  provider, branch, tracking, last fetched, remote and working tree, with the
  folder's entry count and size dimmed below.
- **Previews what is inside.** 180 file types, each a plugin: source in every
  language the workspace holds, images, archives, documents, databases,
  fonts, media. A type may offer more than one view — its own rendering, and
  the file's plain text — and text types can be edited in place, coloured by
  the plugin's own classification of what each part of it is.
- **Handles files the way you expect.** Rename, copy, move, delete, extract an
  archive, create, undo the last one; multi-select, marquee, context menus,
  sortable columns, an address bar with history, drag-and-drop between panes.
  Every operation is confirmed before it runs and journaled afterwards.
- **Hands a repository to the tools you already use.** Open a terminal or
  your configured editor at it, open it on the web, copy its path or remote
  address, or reveal it in the platform's file manager.
- **Finds things across the whole workspace.** A filter field narrows the
  current listing, the status bar's changed-file count is a click away from
  showing just those, and finding a file by name searches every repository
  under the root, not only the one you are looking at.

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
| Working-tree status (is this clone dirty?) | **Built** — an `M` marker on every repository row, a working-tree fact in the tool slot, and a count and filter in the status bar (#535, #582) |
| `explore` command line placeholder (`repo_sphere_explorer`) | **Parked** — an early stub, kept for its own tests, scheduled for removal or repurposing |

## Run it

Install with the setup program on Windows, or the script for your platform.
Either needs a release cut after the install scripts landed (v0.7.0 or
later); v0.6.0 predates the file names they look for.

**Windows** (no administrator rights, no command line): download
**[ReposExplorerSetup.exe](https://github.com/StewartScottRogers/RepoSphereExplorer/releases/latest/download/ReposExplorerSetup.exe)**
and double-click it. It says what it is doing in a window, checks every file
against the release's signed manifest before placing anything, and offers to
start Repos Explorer when it is done. `ReposExplorerSetup.exe --uninstall`
removes it again, and is what the Uninstall button in Settings runs. It is
published with every release cut after it landed; before that, use the
script.

Or the script, which installs the same thing in the same place:

```powershell
irm https://raw.githubusercontent.com/StewartScottRogers/RepoSphereExplorer/main/scripts/install.ps1 -OutFile install.ps1
powershell -ExecutionPolicy Bypass -File install.ps1            # latest release
& "$env:LOCALAPPDATA\Programs\RepoSphereExplorer\RepoSphereExplorerGui.exe"
```

**macOS** (the one drag every Mac user knows):

Download **`ReposExplorer.dmg`** from the
[latest release](https://github.com/StewartScottRogers/RepoSphereExplorer/releases/latest),
open it, and drag **Repos Explorer** onto the **Applications** shortcut beside it.

The application is not signed with an Apple Developer ID (see
[GUIDANCE.md D3](GUIDANCE.md#7-decisions)), so **the first launch has to be
right-click on it, Open, then Open again** in the dialog that appears.
Double-clicking it the first time is refused by Gatekeeper with "Repos
Explorer cannot be opened because the developer cannot be verified". After
that once, it opens like any other application, and it is in Launchpad and
Spotlight with its own icon.

**Linux** - one file, no install step:

```bash
curl -fsSLO https://github.com/StewartScottRogers/RepoSphereExplorer/releases/latest/download/ReposExplorer-x86_64.AppImage
chmod +x ReposExplorer-x86_64.AppImage
./ReposExplorer-x86_64.AppImage
```

The AppImage carries the graphical application, the terminal application,
`service`, the desktop entry and the icon in one file that runs wherever you
put it. It changes nothing on the machine, so there is nothing to uninstall:
delete the file. Being one read-only file, it cannot update itself in place -
`--self-update` says so and names the file to download instead.

**Linux and macOS, with the install script** - the alternative, and the only
way on macOS:

```bash
curl -fsSLO https://raw.githubusercontent.com/StewartScottRogers/RepoSphereExplorer/main/scripts/install.sh
bash install.sh                  # latest release; --tag v0.7.0 for another
RepoSphereExplorerGui            # Linux, via ~/.local/bin
~/Applications/RepoSphereExplorer/RepoSphereExplorerGui   # macOS
```

Each of them downloads the graphical application, the terminal application
(`RepoSphereExplorerTui`) and `service` for your platform, checks each one
against the release's signed update manifest - the same check `--self-update`
uses - before placing anything, and puts them side by side:
`%LOCALAPPDATA%\Programs\RepoSphereExplorer` on Windows,
`~/.local/share/RepoSphereExplorer` on Linux, `~/Applications/RepoSphereExplorer`
on macOS. `-Prefix`/`--prefix` chooses somewhere else. A file that fails the
check is refused and nothing is installed.

On macOS the three go inside `Repos Explorer.app` in that folder - the same
application bundle the disk image carries, so a Mac has one layout however it
was installed - and are linked beside it so the command line can still name
them. `Contents/Info.plist` and `Contents/Resources/AppIcon.icns` are what
give it its name, its version and its icon in Finder, the Dock and Launchpad.

On Windows it then tells the system the application is there, without asking
for administrator rights: **Repos Explorer** appears in the Start menu,
pointing at `RepoSphereExplorerGui.exe` in the install folder, and in
**Settings > Apps**, where the Uninstall button removes it. That button runs
whichever of the two installed it, from a copy left beside the binaries, so
deleting the file you downloaded costs you nothing. What an install is - the
folder, the file names, the receipt, the shortcut and the Settings entry - is
defined once, in `crates/setup/src/layout.rs`, and the script carries a
generated copy of it, so the setup program and the script cannot come to
disagree.

On Linux it tells the desktop the same things, in that desktop's own terms:
a desktop entry at `~/.local/share/applications/reposphereexplorer.desktop`,
so **Repos Explorer** appears in the applications menu and a folder can be
opened with it, and the icon at every size the hicolor icon theme asks for
under `~/.local/share/icons/hicolor`, so the menu entry, the window and the
taskbar show the application's own picture. `update-desktop-database` and
`gtk-update-icon-cache` are run when the machine has them.

To remove it: the Uninstall button, or `ReposExplorerSetup.exe --uninstall` /
`install.ps1 -Uninstall` / `install.sh --uninstall`. That stops the
application and its service and removes what was installed - on Windows the
Start menu entry and the Settings > Apps entry too, on Linux the desktop
entry and the icons. Your journal and Repos Directory configuration stay
unless you add `-Purge -Yes` / `--purge --yes`, which the script does and
the setup program does not.

Every release is installed, run, updated and uninstalled this way on Windows,
Linux and macOS by [`distribution.yml`](.github/workflows/distribution.yml)
before it is published.

**By hand:**
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
and macOS Gatekeeper warn on first run - on macOS, right-click, Open, Open.

Every binary takes a `--self-update` flag that checks the signed manifest
published alongside the Pages site and, if newer, downloads, verifies, and
replaces itself in place - including inside the macOS application bundle,
where it replaces the executable in `Contents/MacOS` where it stands.

### Package managers

Every release publishes a manifest for winget, Scoop and Homebrew alongside
`latest.json`, generated by `package_manifests` from that release's own
signed files - a version cannot reach one of these and not another. None of
the three needs this repository added as a source first: each is installed
straight from the published manifest.

**[winget](https://learn.microsoft.com/windows/package-manager/winget/)**
(Windows):

```powershell
irm https://stewartscottrogers.github.io/RepoSphereExplorer/winget/StewartScottRogers.RepoSphereExplorer.yaml -OutFile RepoSphereExplorer.yaml
winget install --manifest RepoSphereExplorer.yaml
```

**[Scoop](https://scoop.sh/)** (Windows):

```powershell
scoop install https://stewartscottrogers.github.io/RepoSphereExplorer/scoop/repo-sphere-explorer.json
```

**[Homebrew](https://brew.sh/)** (macOS):

```bash
brew install --cask https://stewartscottrogers.github.io/RepoSphereExplorer/homebrew/repo-sphere-explorer.rb
```

**`cargo install`** (any platform with a Rust toolchain and, for the
graphical front end, [Slint's native dependencies](https://github.com/slint-ui/slint)):
every crate a reader installs this way already carries the metadata
`cargo install --git` reads - name, description, license and version:

```bash
cargo install --git https://github.com/StewartScottRogers/RepoSphereExplorer --tag v0.9.0 --bin service service
cargo install --git https://github.com/StewartScottRogers/RepoSphereExplorer --tag v0.9.0 --bin RepoSphereExplorerGui gui
# or, for the terminal front end instead of the graphical one:
cargo install --git https://github.com/StewartScottRogers/RepoSphereExplorer --tag v0.9.0 --bin RepoSphereExplorerTui tui
```

`service` is not started for you this way, the way the setup program and
scripts start it: run it once from a terminal, or launch the front end from
one, before closing that terminal.

## Deferred renames

The pivot renamed the product, the window title and the user-facing strings.
These are not renamed yet, each for a reason:

| Name | Why it stayed |
| --- | --- |
| The repository, `StewartScottRogers/RepoSphereExplorer` | Renaming it moves the Pages site, the release download addresses, and the update manifest that installed copies poll. Needs a redirect plan first |
| Release binaries `RepoSphereExplorerGui` / `RepoSphereExplorerTui` | Installed copies self-update by name against the published manifest; renaming breaks the upgrade path for anyone already running one |
| Settings and journal directory `RepoSphereExplorer/` | Holds live user data. A rename needs a migration that moves the existing directory rather than orphaning it |
| Cargo package `repo_sphere_explorer` (the `explore` placeholder) | Parked rather than renamed, since it is scheduled for removal |
| The Help menu's "About RepoSphereExplorer" item | Missed by the pivot's renaming pass; nothing else depends on the string, so it is a one-line fix whenever a work order touches that menu |

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
| Closing | [`close-linked-issues.yml`](.github/workflows/close-linked-issues.yml) | a pull request closes: closes the issues its `Closes #N` named. Needs `AUTO_MERGE_TOKEN` to fire at all — an event caused by `GITHUB_TOKEN` starts no workflow run |
| Independent review | [`independent-review.yml`](.github/workflows/independent-review.yml) | a work-order pull request opens or updates: a second, separately-invoked agent re-runs the checks and reviews the diff against CLAUDE.md — informational, not yet required |
| Repair | [`repair.yml`](.github/workflows/repair.yml) | checks failed on `main`: diagnoses the run and opens a fix pull request |
| Shipping | [`release.yml`](.github/workflows/release.yml) | tag `v*`: Linux, Windows and macOS binaries attached to a GitHub release, plus `ReposExplorer.dmg` - the macOS application bundle, ready to drag onto Applications |
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
3. Set `AUTO_MERGE_TOKEN`, so a merged pull request closes its work order
   at the moment it lands.

   GitHub starts no workflow run from an event caused by `GITHUB_TOKEN`, and
   closes no linked issue for a merge performed with one. Without this
   secret the factory still merges, but every work order stays open until
   the next sweep of `floor-health-check.yml` - two to five hours, because
   GitHub throttles scheduled workflows on a quiet repository.

   Create a **fine-grained personal access token**, scoped to this
   repository only, with **Contents: read and write**, **Pull requests: read
   and write**, and **Issues: read and write**. Then, from a terminal in
   this repository:

   ```bash
   gh secret set AUTO_MERGE_TOKEN --repo StewartScottRogers/RepoSphereExplorer
   ```

   That command prompts for the value and reads it from your terminal; the
   token never appears in a file, a command line, or a chat.

Already configured on this repository: `main` requires the
`fmt / clippy / test` check with no human review, auto-merge is on, head
branches are deleted after merge, and the `work-order`, `defect` and
`auto-merge` labels exist.

Until step 1 is done the Claude stations are inert and the repository behaves as
an ordinary checked Rust project.

## Built with

All Rust: toolchain 1.98, edition 2024, pinned in
[`rust-toolchain.toml`](rust-toolchain.toml). Versions below are the ones in
[`Cargo.lock`](Cargo.lock).

### The application

| Layer | Framework or crate | What it does here |
| --- | --- | --- |
| Graphical front end | [Slint](https://slint.dev/) 1.17 | The three-pane window, written in `.slint` markup and compiled in by `slint-build`. `i-slint-backend-testing` drives real windows without a display in the integration tests |
| Terminal front end | [Ratatui](https://ratatui.rs/) 0.30 on crossterm 0.29 | The terminal user interface |
| Front end to service | [interprocess](https://crates.io/crates/interprocess) 2.4 | Local sockets - named pipes on Windows - between each front end and the one service process |
| Messages | [serde](https://serde.rs/) 1 with serde_json 1 | Every request and response, as length-prefixed JavaScript Object Notation (JSON) |
| Repository awareness | [gitoxide](https://github.com/GitoxideLabs/gitoxide) component crates: gix-config, gix-ref, gix-odb, gix-revwalk, gix-hash | How far the selected checkout's branch is ahead of or behind its upstream, read from the commit graph on disk. No network client is linked, and nothing runs `git` |
| Filesystem | [trash](https://crates.io/crates/trash) 5, [dirs](https://crates.io/crates/dirs) 6, [ignore](https://crates.io/crates/ignore) 0.4 | Deletes go to the recycle bin; per-user locations for the journal and configuration; finding a name across every repository while honouring `.gitignore` |
| Desktop | [copypasta](https://crates.io/crates/copypasta) 0.10, [open](https://crates.io/crates/open) 5, unicode-segmentation 1 | The system clipboard, handing a repository's web page to the browser, and caret movement by character rather than by byte |
| Self-update | [ureq](https://crates.io/crates/ureq) 3, [ed25519-dalek](https://crates.io/crates/ed25519-dalek) 3, sha2 0.10 | Fetching the release manifest, and verifying every download's signature and digest before it replaces anything |
| Command line placeholder | [clap](https://crates.io/crates/clap) 4 | Argument parsing for the parked `explore` binary |

### The file types

Each of the 180 plugins is its own crate, and most use only serde. The rest
read their format with an established parser: zip, flate2, tar, rars,
sevenz-rust2 and lzma-rs for archives; image and psd for pictures; lopdf for
Portable Document Format (PDF); calamine for spreadsheets; rusqlite for
SQLite databases; arrow, parquet, orc-rust, apache-avro and hdf5-metno for
data files; lofty, mp4 and matroska for media; ttf-parser and wuff for fonts;
object and wasmparser for executables and WebAssembly; x509-parser, der, pem,
pkcs8 and spki for certificates and keys; rpm and ar for packages.

### Building from source

Beyond the pinned Rust toolchain:

- **A C compiler** - rusqlite compiles its own copy of SQLite.
- **CMake** - hdf5-metno builds the HDF5 library from source.
- **On Linux**, the development packages Slint needs for X11 and input, as
  listed in [`release.yml`](.github/workflows/release.yml).

### The factory

[GitHub Actions](.github/workflows/) runs every station: the three checks and
a release build in [`ci.yml`](.github/workflows/ci.yml), with
`Swatinem/rust-cache` and `cargo-llvm-cov` for coverage;
[`rustsec/audit-check`](https://github.com/rustsec/audit-check) for dependency
advisories; [`anthropics/claude-code-action`](https://github.com/anthropics/claude-code-action)
for the factory shift, the independent review, repair, and answering `@claude`
mentions; and
`softprops/action-gh-release` to publish releases. The site and its history
film are built with [Gource](https://gource.io/) and ffmpeg and served by
GitHub Pages.

## Local commands

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo run -p gui                 # opens at the configured Repos Directory
```

Toolchain is pinned in [`rust-toolchain.toml`](rust-toolchain.toml) (1.98,
edition 2024), so local builds and the factory's match.

## License

MIT — see [LICENSE](LICENSE).
