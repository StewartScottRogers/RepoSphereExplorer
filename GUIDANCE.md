# Repos Explorer — Design Guidance

**Status: D1–D10 settled, ready for a build order.** This is the source document
the factory builds from. Edit it directly; every work order should trace back to
a line in here. If this document and the code disagree, this document is wrong —
fix it here first, then let the factory change the code.

Sections are numbered to match the original brief. Decisions D1–D10 in [§7](#7-decisions)
are settled below and gate the build order in [§8](#8-build-order).

## 0. What this application is for

A **Repos Explorer**: a purpose-built front door to the development
workspace. Its subject is the set of local working directories that source
control systems — GitHub, GitLab, Bitbucket, Azure DevOps, plain Git remotes,
and others later — check code out into. It opens at one configured **Repos
Directory**, it knows which of the folders inside it are working copies, and it
says where each one came from.

It is **not** a general-purpose file explorer. Browsing the whole filesystem is
not the goal, and no feature is justified by "a file explorer would do this".
The question a proposal has to answer is whether it helps somebody reach and
understand the repositories they work in.

That scope was set on 2026-09-08 and is recorded in
[DECISIONS.md](DECISIONS.md); §2.5 and decisions D7–D10 below carry the detail.

---

## 1. Operating model — a dark factory on GitHub

Work enters as a GitHub issue, machines do the rest, a signed release comes out.
No step assumes a human is watching.

| Station | Trigger | Output |
| --- | --- | --- |
| Inspection (`ci.yml`) | every push and pull request | fmt, clippy `-D warnings`, tests, release build, `cargo audit` |
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
indexing, file operations, plugin execution, caching. The **terminal user
interface (TUI)** and **graphical user interface (GUI)** front ends render state
and send intents. They hold no business rules, and no
front end reaches the filesystem directly for anything the service can answer.

```
  ┌────────────┐        ┌────────────┐
  │    TUI     │        │    GUI     │    thin: render + intents
  │ (Ratatui)  │        │  (Slint)   │
  └─────┬──────┘        └─────┬──────┘
        │                     │
        │   local inter-process communication   <-- trust boundary
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

The inter-process communication (IPC) boundary is a trust boundary and the
parsers are the attack surface.

1. **Local transport only.** Unix domain socket with `0600`, or a Windows named
   pipe with an explicit discretionary access control list (DACL) limited to the
session owner. No Transmission Control Protocol (TCP) listener, not
   even on loopback — loopback is reachable by every local user and every
   browser on the machine.
2. **Authenticate the peer.** Verify the connecting process's user identifier
   (UID, through `SO_PEERCRED`) or security identifier (SID) before serving a
   single request. A socket without this
   is a local privilege-escalation primitive.
3. **Every request is hostile.** Canonicalize and resolve paths (symlinks,
   junctions, `..`) before use; operate on open handles rather than re-resolving
   paths — the time-of-check-to-time-of-use (TOCTOU) race; never invoke a
   shell; never interpolate a path into a command.
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

### 2.2 Terminal front end (Ratatui)

Cross-platform, and a good citizen inside another multiplexer such as herdr:

- Never fight for the alternate screen; restore terminal state on every exit
  path, including panic and the host pane closing.
- Handle resize continuously; the host pane changes size without warning.
- Mouse capture must be releasable so the host's own selection still works.
- Degrade cleanly to 16 colours and to plain American Standard Code for
  Information Interchange (ASCII) text; do not require a nerd font.

### 2.3 Graphical front end (Slint)

Native *feel* per platform — two behaviour profiles, not one averaged one.

| | Windows | macOS |
| --- | --- | --- |
| Rename | `F2` | `Return` |
| Delete | `Del`, to Recycle Bin | `Cmd+Delete`, to Trash |
| Copy / paste | `Ctrl+C` / `Ctrl+V` | `Cmd+C` / `Cmd+V` |
| Preview | `Space`, optional | `Space` quick look, expected |
| Modals | dialogs | sheets |
| Chrome | title bar, menu bar | traffic lights, unified toolbar |

### 2.4 The three panes

Windows Explorer-inspired in its handling, mouse-first, keyboard as a peer —
but pointed at the Repos Directory rather than at the machine:

1. **Folders view** — tree, expand/collapse, drag targets. Its root is the
   Repos Directory, not a list of drives: this is a workspace, not a volume
   browser.
2. **Folder contents view** — list / details / icons, sortable columns, marquee
   select, drag-and-drop, context menus, inline rename. A child of the root
   that is a working copy is marked as one and names its provider; a folder
   that is not stays visible and looks different.
3. **File pane** — supplied entirely by the file-type plugin: view, edit, and
   the operations that type offers. A type may offer more than one view of
   the same file - its own rendering, and the file's plain text where it has
   one - which the pane switches between; the plugin names them and decides
   how many there are. For a repository, the pane reports the provider, the
   branch checked out, and the remote it tracks.

Splitters are draggable and persisted. Everything reachable by mouse is also
reachable by keyboard.

### 2.5 The Repos Directory

The anchor the whole application is arranged around.

- **One configured root.** For example `Z:\repos` on Windows. Every launch
  opens there, whatever the user was looking at when the application last
  closed. There is deliberately no last-location session restore: a front door
  that opens somewhere different each morning is not a front door.
- **Per machine, persisted, editable.** The path is stored with the rest of the
  user's settings and can be changed from inside the application. Roots are
  stored as a list with exactly one marked active, so several roots can be
  supported later without changing the stored shape.
- **Cross-platform default.** `Z:\repos` on Windows, `~/repos` elsewhere. When
  nothing is configured, the first run offers the default and takes what the
  user gives it.
- **Repository awareness.** The immediate children of the root are treated as
  source control working directories. A real clone is detected by its `.git`
  marker — never guessed from a name — and the provider is read from the
  remote address in the repository's own configuration. Branch comes from the
  same place. Whether the working tree has uncommitted changes is read from
  the checkout's own index, for the **selected** repository only: a listing
  of forty checkouts cannot afford a pass over forty sets of tracked files,
  and the one a reader is looking at is a pass they asked for. Tracked files
  only - deciding whether an untracked file is ignored needs the ignore
  rules - so the wording says "tracked" and never "clean".
- **The boundary is soft.** The root is home base, not a cage: navigating above
  or outside it is allowed. Soft against hard is one setting in one place
  (D8), so it can be reconsidered without hunting through the code.

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
- Nothing blocks the user interface thread. Every long operation is
  cancellable from the user interface.

### 3.4 One format inside another

Some formats are a narrower reading of a format already built. An npm lock
file is JSON. A Kubernetes manifest is YAML. A Java archive is a zip.

Both plugins recognise such a file, and the general one almost always owns
the extension — so §3.3's hint hands the file to the general plugin
whatever order they are registered in. That is the hint doing its job in a
case it was not written for: it exists to settle a tie between *siblings*
that have no magic bytes and genuinely overlap, which is what a C file
opening as Rust needed (#272).

So a plugin names what it refines, and the dispatch drops the refined
plugin before the hint is applied. A specialisation beats the format it
specialises; siblings are still settled by extension.

### 3.5 Folder plugins

A folder is a subject too, and it answers a different question from a file.

A file has exactly one type. Two plugins claiming one file is a defect, which
is why §3.3's extension hint exists to settle it — a C file opening as Rust
was a real bug (#272), not a hypothetical one.

Folder facts stack. This repository's own root is a source control working
copy **and** a Cargo workspace, and neither description is the wrong one. So
folder plugins have their own trait pair and their own registry, and dispatch
collects **every** plugin that recognises a folder rather than picking a
winner. Their lines are added below what the folder already reports; a folder
that is a project is still a folder, and a reader still wants to see what is
inside it.

Sniffing a folder reads the names of the entries directly inside it, not a
prefix of bytes. A folder has no bytes, and every project marker there is —
`Cargo.toml`, `package.json`, `go.mod`, `pom.xml` — is a file name.

**The selected folder only.** A `.git` check is one `metadata` call per row; a
folder sniff is a full directory read per row, and a Repos Directory holding
two hundred folders would pay it two hundred times before a single row drew.
Whether a listing can afford a project column is a separate question that this
does not answer, and the omission is deliberate rather than an oversight.

Reading, never driving, exactly as D10 requires: a folder plugin reads the
manifest the author wrote and never runs the build tool. Resolving a manifest
into what would actually build needs the registry, the lock file and the
network, which is a different job from describing a folder.

## 4. Distribution — GitHub is the whole supply chain

### 4.1 The web page

A **GitHub Pages** site, published on every release, rather than the repo README:
a README cannot play a Gource film or host a live dashboard. The site carries
operating-system-detected download buttons, the film, the statistics and the
update manifest.

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

A stable, versioned `latest.json` at a fixed uniform resource locator (URL) —
version, per-target addresses,
sizes, hashes, signatures, minimum-upgradable-from version — plus the GitHub
Releases application programming interface (API). Any script or package manager
can discover and fetch without scraping HyperText Markup Language (HTML). The manifest URL never moves.

## 5. Proposed workspace layout

A cargo workspace:

```
crates/
  protocol/      inter-process communication message types, shared by all
  plugin-api/    the two plugin traits
  service/       the fat process: filesystem, repositories, operations,
                 plugin cores, configuration
  tui/           Ratatui terminal front end
  gui/           Slint graphical front end
  cli/           legacy `explore` placeholder, kept for its tests
  updater/       signature-verified self-update
  plugins/       one crate per file type, each: core + presentation
samples/         one fixture directory per plugin
site/            GitHub Pages source: downloads, film, statistics, latest.json
```

## 6. Non-goals

Stated so the factory does not drift into them.

- **Not a general-purpose file explorer.** Browsing the machine is not the
  product. A feature earns its place by helping somebody reach or understand
  the repositories they work in.
- **No source control operations.** Clone, fetch, pull, commit, merge and the
  rest belong to the tools that already do them well. This application detects
  and describes; it does not drive (D10).
- **No mounting of network file sharing protocols.** Server Message Block
  (SMB), Network File System (NFS), WebDAV and their kin are the operating
  system's job. Where the operating system has already mounted one — a mapped
  drive such as `Z:` — a Repos Directory on it is an ordinary path and works
  like any other. Reaching one that the operating system has *not* mounted is
  out of scope.
- **No cloud sync, no remote browsing** of a provider's servers: this reads the
  working copies on this machine, not the repositories on a host.
- **No third-party binary plugins, no mobile front end, no in-app package
  management** beyond the updater, and **no telemetry of any kind.**

## 7. Decisions

All settled. Nothing here is open; the build order in §8 proceeds. D7 to D10
were settled on 2026-09-08 and are recorded, with their reasoning, in
[DECISIONS.md](DECISIONS.md). They are revisitable: each names the one place a
change would have to be made.

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

**D6 — Editing scope for v2: GUI usability parity with Windows File Explorer.**
The three-pane GUI (§2.2's TUI is not in scope for this decision) implements
only D4's v1 operations today, and only a fraction of §2.3/§2.4's own
already-settled behaviour: no multi-select, no context menus, no address bar,
no sortable columns, no drag-and-drop, no create/undo. Settled 2026-09-06:

- [x] Extend D4's v1 operations (rename, copy, delete, extract) to also cover
  create (new folder, new empty file), move to an arbitrary destination (not
  only a same-directory rename), and undo of the immediately preceding
  operation.
- [x] Build out §2.3/§2.4 as already specified but not yet implemented:
  platform-appropriate keyboard shortcuts per the §2.3 table, right-click
  context menus, sortable columns, marquee/multi-select and batch operations,
  internal (pane-to-pane) drag-and-drop, inline rename, draggable and
  persisted splitters, an address bar with back/forward navigation, a status
  bar (item count/size), and a properties view (size, modified time).
- [x] Add a directory-contents filter box (typed substring match against the
  current folder's listing) - a new addition beyond §2.4's original wording,
  since Explorer users expect it.
- Deferred, explicitly not in this scope: dragging files in from the operating
  system's shell (import from outside the application), recursive/whole-tree
  search, thumbnails beyond what a plugin's own preview already renders.
  Network and cloud locations remain excluded by §6.

D6 is kept as it was settled on 2026-09-06, under the general-purpose mission
that D7 replaced. What survives it is the *handling* — a person who knows
Windows File Explorer should not have to learn new gestures. What does not
survive is the scope: parity was never a licence to browse the whole machine,
and after D7 the panes are pointed at the Repos Directory (§2.5).

**D7 — What the application is for.** Settled 2026-09-08, replacing the
general-purpose file explorer this began as.

- [x] A Repos Explorer: the front door to the local working directories that
  source control systems check code out into, anchored on one configured Repos
  Directory (§2.5). Every launch opens there; there is no last-location session
  restore. Windows, macOS and Linux all stay supported, with a per-platform
  default root and a first-run prompt.

**D8 — Boundary around the Repos Directory.** Is the root a home base or a
cage?

- [x] Soft. The application opens at the root and nothing prevents navigating
  outside it. Revisitable: the policy is one configuration point, so a hard
  boundary is a change in one place rather than a change everywhere.

**D9 — How many roots.** One Repos Directory, or several at once?

- [x] One active at a time, stored as a list with one marked active.
  Revisitable: supporting several is then a user interface change, not a
  migration of stored settings.

**D10 — Source control operations.** Does the application drive Git, or only
read it?

- [x] Read only, this phase: detect a working copy, identify its provider, show
  its branch and status. Clone, fetch, pull and commit are delegated to the
  tools that own them. Revisitable: the detection layer is built so operations
  can sit behind it, and working-tree status is the first thing that will need
  the same tree walk they do.

## 8. Build order

1. Convert to the §5 workspace; CI green on the empty crates.
2. `protocol` plus a service that answers one request (list a directory) and a
   TUI that renders it.
3. `plugin-api` and the text plugin, both halves, end to end.
4. Three-pane TUI over real directories, with cancellable listing.
5. GUI to parity with the TUI, then the Pages site, updater, film, statistics.
6. Realignment to the Repos Explorer mission (D7–D10): the Repos Directory
   anchor and its first-run experience, repository detection with provider and
   branch, and the documentation that describes all of it.

Each numbered item is one work order with its own acceptance checks.
