# RepoSphereExplorer — Design Guidance

**Status: D1–D5 settled, ready for a build order.** This is the source document
the factory builds from. Edit it directly; every work order should trace back to
a line in here. If this document and the code disagree, this document is wrong —
fix it here first, then let the factory change the code.

Sections are numbered to match the original brief. Decisions D1–D5 in [§7](#7-open-decisions)
are settled below and now gate the build order in [§8](#8-build-order-once-d1d5-are-settled).

---

## 1. Operating model — a dark factory on GitHub

Work enters as a GitHub issue, machines do the rest, a signed release comes out.
No step assumes a human is watching.

| Station | Trigger | Output |
| --- | --- | --- |
| Inspection (`ci.yml`) | every push and PR | fmt, clippy `-D warnings`, tests, release build, `cargo audit` |
| Assembly (`claude.yml`) | issue labelled `work-order`, `@claude` comment | a pull request |
| Night shift (`factory-shift.yml`) | nightly 03:00 UTC | oldest unbuilt work order becomes a PR |
| Repair (`repair.yml`) | CI red on `main` | a fix PR, or an issue if the cause is external |
| Restocking (`dependabot.yml`) | weekly | dependency PRs, auto-merged when green |
| Dispatch (`auto-merge.yml`) | Dependabot or `auto-merge` label | merge once required checks pass |
| Shipping (`release.yml`) | tag `v*` | platform binaries on a GitHub release |

**Consequences that shape everything below.** A work order must state an
observable outcome and the acceptance checks that prove it, because checks — not
a reviewer's taste — decide whether work landed. Anything the factory cannot
verify automatically it must not build unattended. Any design that makes a unit
of work large or unverifiable is the wrong design for this repository.

## 2. Architecture — one brain, two faces

A **fat service** process owns all logic: filesystem traversal, file parsing,
indexing, file operations, plugin execution, caching. The **TUI** and **GUI**
front ends render state and send intents. They hold no business rules, and no
front end reaches the filesystem directly for anything the service can answer.

```
  ┌────────────┐        ┌────────────┐
  │    TUI     │        │    GUI     │    thin: render + intents
  │ (Ratatui)  │        │  (Slint)   │
  └─────┬──────┘        └─────┬──────┘
        │                     │
        │   local IPC (socket / named pipe)   <-- trust boundary
        └──────────┬──────────┘
              ┌────▼─────┐
              │ Service  │                  fat: all logic
              │ + plugin │
              │   cores  │
              └────┬─────┘
                   │
          filesystem, untrusted file content
```

One service instance may serve both front ends at once. The service is the only
process that touches user data, so it is the only process that has to be right
about safety.

### 2.1 Security model

The IPC boundary is a trust boundary and the parsers are the attack surface.

1. **Local transport only.** Unix domain socket with `0600`, or a Windows named
   pipe with an explicit DACL limited to the session owner. No TCP listener, not
   even on loopback — loopback is reachable by every local user and every
   browser on the machine.
2. **Authenticate the peer.** Verify the connecting process's UID
   (`SO_PEERCRED`) or SID before serving a single request. A socket without this
   is a local privilege-escalation primitive.
3. **Every request is hostile.** Canonicalize and resolve paths (symlinks,
   junctions, `..`) before use; operate on open handles rather than re-resolving
   paths (TOCTOU); never invoke a shell; never interpolate a path into a command.
4. **Parsers are sandboxed by policy.** Memory-safe Rust parsers only. No C
   bindings without an isolation story. Every parse runs under a time, memory
   and output-size limit. No parser gets network access. A preview never
   executes anything the file asks to have executed — no macros, no embedded
   scripts, no external entity resolution, no automatic archive extraction.
5. **Least privilege, explicit destruction.** The service runs as the user and
   never elevates. Destructive operations (delete, overwrite, bulk rename) need
   an explicit confirmed intent carrying the exact target set, and are journaled
   so the action can be described after the fact.

**Threat model in one line:** the adversary is a file, not a network attacker.
The realistic compromise is a malicious document the user merely previews.

### 2.2 TUI front end (Ratatui)

Cross-platform, and a good citizen inside another multiplexer such as herdr:

- Never fight for the alternate screen; restore terminal state on every exit
  path, including panic and the host pane closing.
- Handle resize continuously; the host pane changes size without warning.
- Mouse capture must be releasable so the host's own selection still works.
- Degrade cleanly to 16 colours and to plain ASCII; do not require a nerd font.

### 2.3 GUI front end (Slint)

Native *feel* per platform — two behaviour profiles, not one averaged one.

| | Windows | macOS |
| --- | --- | --- |
| Rename | `F2` | `Return` |
| Delete | `Del`, to Recycle Bin | `Cmd+Delete`, to Trash |
| Copy / paste | `Ctrl+C` / `Ctrl+V` | `Cmd+C` / `Cmd+V` |
| Preview | `Space`, optional | `Space` quick look, expected |
| Modals | dialogs | sheets |
| Chrome | title bar, menu bar | traffic lights, unified toolbar |

### 2.4 The three-pane explorer

Windows Explorer-inspired, mouse-first, keyboard as a peer:

1. **Folders view** — tree, expand/collapse, drag targets, roots for drives,
   home and pinned locations.
2. **Folder contents view** — list / details / icons, sortable columns, marquee
   select, drag-and-drop, context menus, inline rename.
3. **File pane** — supplied entirely by the file-type plugin: view, edit, and
   the operations that type offers.

Splitters are draggable and persisted. Everything reachable by mouse is also
reachable by keyboard.

## 3. File types as plugins

Every format is a plugin owning its icon, thumbnail and graphics, viewer, editor
and offered operations. Unknown types fall back to text/hex. The catalogue grows
forever, so adding a type must be a small, mechanical, verifiable unit of work —
"support `.parquet`" is exactly one work order.

### 3.1 Two halves, one crate

The brief says plugins compile into the front ends; the architecture says all
logic lives in the service. Both hold if a plugin is one crate with two faces:

- **Core half** (linked into the service): sniff, parse, extract, thumbnail,
  operate. Sees untrusted bytes. Runs under the §2.1 limits.
- **Presentation half** (linked into each front end): icon, colours, widget
  layout, edit surface, keybindings. Sees only structured data the core
  produced, never raw file bytes.

A shared `plugin-api` crate defines both traits so the halves cannot drift. A
plugin author writes one crate; each binary picks up the right half by feature
flag.

### 3.2 Compiled in, not loaded

Static linking. No `dlopen`, no runtime WASM, no third-party drop-ins. The trade
is deliberate: maximum speed and no dynamic-code attack surface, paid for with a
release per new format — cheap, because releases are automated and updates are
automatic. Registration is a generated static table, so dispatch is a match
rather than a lookup, and unused plugins can be feature-gated out of a build.

### 3.3 Speed rules

- Sniffing is content-based (magic bytes) with the extension as a hint only, and
  reads a bounded prefix — never the whole file.
- Directory listing streams; the first screen renders before the walk finishes.
- Thumbnails and parses are cancellable, and cached by (path, mtime, size).
- Nothing blocks the UI thread. Every long operation is cancellable from the UI.

## 4. Distribution — GitHub is the whole supply chain

### 4.1 The web page

A **GitHub Pages** site, published on every release, rather than the repo README:
a README cannot play a Gource film or host a live dashboard. The site carries
OS-detected download buttons, the film, the statistics and the update manifest.

### 4.2 Auto-update, by many means

In-app updater plus package managers: winget, Homebrew tap, Scoop,
`cargo install`. Non-negotiable: **every update is signature-verified before it
is applied** (minisign or cosign, keys held as repository secrets), or the
updater becomes a malware delivery channel. Updates stage and apply atomically,
with a rollback path if the new binary fails to start.

Platform reality: macOS needs Developer ID signing plus notarization or
Gatekeeper blocks the download; unsigned Windows binaries trip SmartScreen on
every download until reputation accrues. See decision D3.

### 4.3 Gource film per release

A workflow renders repository history to video after each tag and places it at
the top of the Pages site. It needs full history (`fetch-depth: 0`), a few
minutes of runner time, and a size budget — the film grows with the repo, so cap
resolution and length, and keep only the newest few.

### 4.4 Development statistics

Regenerated by the same workflow, never by hand: commits and contributors over
time, work orders opened and closed, CI pass rate and duration, release cadence,
binary size per platform, plugin count and supported formats.

### 4.5 Machine-pullable updates

A stable, versioned `latest.json` at a fixed URL — version, per-target URLs,
sizes, hashes, signatures, minimum-upgradable-from version — plus the GitHub
Releases API. Any script or package manager can discover and fetch without
scraping HTML. The manifest URL never moves.

## 5. Proposed workspace layout

The repository is a single CLI crate today; this design is a cargo workspace.

```
crates/
  protocol/      IPC message types, versioned, shared by all
  plugin-api/    the two plugin traits + registration macro
  service/       the fat process: fs, index, ops, plugin cores
  tui/           Ratatui front end
  gui/           Slint front end
  plugins/
    text/  image/  archive/  pdf/  directory/    each: core + presentation
site/            GitHub Pages source: downloads, gource, stats, latest.json
```

## 6. Non-goals

Stated so the factory does not drift into them: no remote or network browsing,
no cloud sync, no third-party binary plugins, no mobile front end, no in-app
package management beyond the updater, no telemetry of any kind.

## 7. Open decisions

All settled as of 2026-09-01. Nothing here is open any longer; the build order
in §8 proceeds.

**D1 — Plugin split.** Core-in-service plus presentation-in-front-end, as in §3.1?

- [x] Yes, as written

**D2 — Slint licence.** The repository is MIT today; worldwide distribution
forces the choice.

- [x] Royalty-free licence, accepting its attribution conditions

**D3 — Signing budget.** Apple Developer Program (~$99/yr) and a Windows
code-signing certificate?

- [x] Neither — ship unsigned, document the warnings

**D4 — Editing scope for v1.**

- [x] View plus operations only (rename, copy, delete, extract)

**D5 — First five formats, to prove the architecture.**

- [x] text, image, archive, PDF, directory-as-file

## 8. Build order once D1–D5 are settled

1. Convert to the §5 workspace; CI green on the empty crates.
2. `protocol` plus a service that answers one request (list a directory) and a
   TUI that renders it.
3. `plugin-api` and the text plugin, both halves, end to end.
4. Three-pane TUI over real directories, with cancellable listing.
5. GUI to parity with the TUI, then the Pages site, updater, film, statistics.

Each numbered item is one work order with its own acceptance checks.
