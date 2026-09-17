# Architecture decisions

One entry per decision that shaped the application, in the order they were
settled. Each says what was decided, why, what it costs, and what would
have to change to revisit it. [GUIDANCE.md](GUIDANCE.md) is the design the factory builds
from; this file is the record of how that design got the way it is.

Decisions D1 to D6 predate this record and live in
[GUIDANCE.md §7](GUIDANCE.md#7-decisions), where they were settled.

---

## D7 — The application is a Repos Explorer, not a file explorer

**Date:** 2026-09-08
**Status:** Accepted
**Supersedes:** the general-purpose mission that D1 to D6 were settled under

### Decision

The application's sole purpose is managing access to the local working
directories that source control systems — GitHub, GitLab, Bitbucket, Azure
DevOps, plain Git remotes, and others later — check code out into. It is a
purpose-built front door to the development workspace, not a browser for the
whole filesystem.

Three things follow, and are settled with it:

1. **The Repos Directory anchor.** One configured root, for example
   `Z:\repos` on Windows. Every launch opens there, wherever the user was
   when the application last closed. There is no last-location session
   restore, and none is to be added. The path is per machine, persisted, and
   editable by the user.
2. **Repository awareness.** The immediate children of the root are treated
   as source control working directories. A real clone is detected by its
   `.git` marker rather than guessed from a name, and the design must allow
   provider, branch and working-tree status to be surfaced for each one.
   Folders that are not repositories stay visible and look different.
3. **Cross-platform stays.** Windows, macOS and Linux. The default root is
   expressible per platform — `Z:\repos` on Windows, `~/repos` elsewhere —
   with a prompt on first run when nothing is configured.

### Why

A general-purpose file explorer competes with the one already installed on
every machine, and has no natural boundary: every feature the operating
system's own explorer has becomes a gap. A Repos Explorer has a subject.
Its features are answerable to one question — does this help somebody reach
or understand the repositories they work in — and that question can be
answered no.

### What it costs

Work already built against the old mission stays, but stops being an end in
itself: the file-type plugins are now there to preview what is *inside* a
repository, not to browse a disk. Anything that only made sense for browsing
a machine is parked (see [README.md](README.md#what-changed-in-the-pivot)).

### To revisit

Rewrite this entry and GUIDANCE.md §0 together. Every other document takes
its mission from those two.

---

## D8 — The boundary around the Repos Directory is soft

**Date:** 2026-09-08
**Status:** Accepted, revisitable

### Decision

The Repos Directory is home base, not a cage. The application opens there
and nothing prevents navigating above or outside it.

### Why

A hard sandbox would refuse ordinary, reasonable moves — opening a sibling
checkout somewhere else, following a path out of a symbolic link, looking at
a file the user dragged in from elsewhere — and would have to grow an escape
hatch almost immediately. Soft is the setting that can be tightened later
without stranding anybody; hard is the one that generates complaints on the
first day.

### What it costs

Nothing in the application enforces that the user is inside the workspace, so
"where am I" has to be visible rather than guaranteed. The address bar and
the root marker carry that weight.

### To revisit

The policy is one configuration point. Changing it to a hard sandbox means
changing that point and the code that consults it — not hunting through
navigation, drag-and-drop and the address bar for places that assume a soft
boundary.

---

## D9 — One active Repos Directory, stored as a list

**Date:** 2026-09-08
**Status:** Accepted, revisitable

### Decision

One Repos Directory is active per machine. Roots are nevertheless stored as
a list with exactly one marked active.

### Why

One root is what a person needs today, and one root keeps the user interface
free of a concept — root switching — that nothing yet uses. Storing a list
anyway costs one field and removes the migration that would otherwise be the
price of supporting several later.

### What it costs

Code that reads the configuration has to pick the active entry rather than
read a single path, and has to cope with a list holding none or several.
That cost is paid once, in the configuration layer.

### To revisit

Supporting several roots at once becomes a user interface change — how the
user sees and switches them — with the stored shape already correct.

---

## D10 — This phase reads source control, it does not drive it

**Date:** 2026-09-08
**Status:** Accepted, revisitable

### Decision

Repository-aware navigation only: detect a working copy, identify the
provider it came from, and show its branch and status. Clone, fetch, pull,
commit, merge and the rest are out of scope and delegated to the tools that
already do them.

### Why

Driving source control is a large surface with real consequences for a
user's work — a wrong pull is not a wrong preview. The detection layer is
what everything else would need first anyway, and it is worth having on its
own: knowing at a glance which of forty folders are clones, and where they
came from, is the point of the application.

### What it costs

The application will be asked for a "pull" button, and the answer for now is
no.

**Amended 2026-09-08 by work order #304.** This entry deferred working-tree
status along with the operations, on the grounds that reporting whether a
clone is dirty needs the same walk of the work tree. That was true of the
listing and not of the selected repository: read from the checkout's own
index, for one repository at a time, the answer costs a pass over its
tracked files and nothing more. Status is now reported in the File pane.
What D10 still settles is unchanged - this application reads and never
drives, and clone, fetch, pull and commit remain out of scope.

### To revisit

Operations sit behind the detection layer that D10 builds. Adding them means
adding requests to the service and controls to the front ends, not
rearranging how repositories are found.

---

## D11 — The factory merges with a user token, not `GITHUB_TOKEN`

**Date:** 2026-09-08
**Status:** Accepted

### Decision

`auto-merge.yml` merges with `AUTO_MERGE_TOKEN`, a fine-grained personal
access token scoped to this repository, falling back to `GITHUB_TOKEN`
when the secret is absent.

### Why

GitHub starts no workflow run from an event caused by `GITHUB_TOKEN`, and
closes no linked issue for a merge performed with one. Both consequences
were live here: every work order this factory shipped stayed open after
its pull request merged, and `close-linked-issues.yml` - written to close
them on the merge event - never ran at all. Pull request #292 merged and
produced no run of it.

The alternatives were to accept a two-to-five-hour closure window from the
scheduled sweep, or to hang the closure off an event that fires slightly
before the merge and would usually do nothing. Neither is a mechanism; both
are a hope. A token that triggers workflows is the documented way to make
an automated merge behave like a merge.

### What it costs

A credential to store, rotate and audit - the thing decision #286 was
trying to avoid before it was known that avoiding it did not work. It is
scoped to one repository and to three permissions. Merges will show as the
token's owner rather than as `github-actions`, which is honest: a person
owns this factory.

It also un-suppresses the workflows that a merge to `main` should have been
starting all along, so expect runs that were previously silent.

### To revisit

Remove the secret. The fallback in `auto-merge.yml` takes over, the factory
keeps merging, and the sweep goes back to closing work orders in its own
time. Nothing else has to change.

## D12 — Folder facts stack; file facts do not

**Settled 2026-09-09.**

A file has exactly one type. The whole file plugin dispatch is built on that:
one `sniff` wins, and the extension hint exists only to settle which of two
overlapping claims is right. Two plugins claiming one file was a defect, and
a real one — a C file with a top-level `struct` opened as Rust, and every Java
file opened as Perl (#272).

A folder is not like that. This repository's own root is a source control
working copy and a Cargo workspace at the same time, and there is no honest
way to pick one of those as the answer. A monorepo is worse and more ordinary:
the checkout at the top, a Go module in one subfolder, a Node package in
another, each of them also a plain folder full of files.

So folder plugins get their own trait pair (`FolderCore`, `FolderPresentation`)
and their own registry, and the dispatch **collects** rather than selects.
Their lines are appended to what the folder already reports rather than
replacing it — settled with the requester on 2026-09-09, and the reason is
that a folder that is a project is still a folder.

**What it would take to revisit.** The collecting is one function,
`service::folder_plugins_among`, and the union is assembled in one place, the
`is_dir` arm of `service::view_file`. A test with two matching test doubles
fails if the `filter` there is ever replaced by a `find`.

**What is deliberately not decided.** Whether a *listing* can show a project
column. A `.git` check is one `metadata` call per row; a folder sniff is a
full directory read per row. Today only the selected folder is sniffed. See
GUIDANCE.md §3.4.

## D13 — A specialisation beats the format it specialises

**Settled 2026-09-09.**

The extension hint was added because source languages have no magic bytes
and genuinely overlap: `struct` belongs to C, C++, Rust, Swift and
Solidity alike, and a C file was opening as Rust (#272). Among siblings
like those, the extension is the only honest tiebreak there is.

It does something else entirely when one plugin is a narrower reading of
another. An npm lock file *is* JSON; `json` owns the extension; so `json`
won, whatever the registration order said. About twenty of the hundred
plugins filed as #316 to #415 are specialisations of a format already
built — JSON, YAML, TOML, XML and zip each have several — and every one of
them would have lost the same way.

Teaching the general plugin about its specialisations was the alternative,
and it puts npm's business in the JSON plugin and Kubernetes' in the YAML
plugin. So the specialisation declares what it refines instead, and
`service::most_specific` drops the refined plugin before the extension
hint is applied.

**What it would take to revisit.** One function,
`service::most_specific`, and one trait method with a default,
`PluginCore::specialises`. A test with three doubles — a general plugin
owning the extension, a specialisation, and a sibling refining nothing —
fails if the drop is removed, and was checked by removing it.

**What is deliberately unchanged.** The hint itself. A plugin that
specialises nothing behaves exactly as it did, which is what keeps the
#272 resolution intact.

## D14 — The File pane is an editor, within a written boundary

**Settled 2026-09-13.**

The pane could already edit a file: Edit, type, Save, written back
through the service. That was a *fix a typo* affordance and it sat
comfortably inside an application for reaching and understanding
repositories (D7, D10).

Syntax colouring does not sit inside it. Slint 1.17.1's text-entry
widget has one `color` property and no per-range styling, and its
`StyledText` element, which does colour spans, cannot be typed into. So
colouring what somebody is editing means writing the editing surface by
hand: caret, selection, undo, scrolling, virtualisation - and losing
input method editor (IME) composition, screen-reader accessibility and
right-to-left text until each is put back.

That is a different product living inside this one, and left unstated
it would keep asking for more: find and replace, bracket matching,
folding, completion. So the boundary is written down instead, in
GUIDANCE.md §3.6, and a proposal outside it has to argue against a line
rather than against a mood.

What made it worth doing anyway is that the expensive half is already
built. A hundred and eighty plugins parse these formats today. The
Roslyn C# compiler platform's lesson is that classification belongs to
the front end that already parses - so the plugin returns spans and the
pane paints them, which is what §3 has always said about icons and
views. Nothing new is being invented; an existing idea is being applied
once more.

**What it would take to revisit.** The classification half stands on
its own and would survive: `PluginPresentation::classify` defaults to
no spans, so a pane that stopped painting them would simply read as it
did before. The editing surface is the reversible part - the File pane
chooses between the hand-written editor and Slint's `TextEdit` in one
place, and going back means choosing the other and losing colour while
typing.

**What this supersedes.** GUIDANCE.md §7's D4 settled the editing scope
for v1 as "view plus operations only", and D6 extended it to the
operations Windows File Explorer has. Neither anticipated editing a
file's text, which the pane nonetheless grew. This says what that is
and where it stops; §3.6 is the boundary D4 and D6 never drew.

**What is deliberately unchanged.** Where a save goes. The service is
still the only process that touches the filesystem, `save_file_edit`
still sends `Request::WriteFile`, and nothing about the editing surface
alters D10: it reads and writes the file a reader chose, and runs
nothing.

## D15 — Panes dock or float; the right-hand pane hosts tools

**Settled 2026-09-17.**

The window has been three fixed columns since the GUI began: Folders,
Contents, and a File pane that showed one file plugin's view of the
selection. That third pane is renamed and widened. It is no longer "the
File pane" — it is a **tool slot**, home to whichever tool applies to
what is selected. The editor (§3.6) is its first tool, compiled in
exactly as file-type plugins are (§3.2); a certificate tool is its
second, and it is **read-only**: it finds the certificates committed
under the Repos Directory and reports which are expired or expiring,
with no issuing, renewing, revoking or deploying. Those four are
lifecycle operations - driving rather than reading, the distinction D10
already draws for source control - and renewing or deploying a
certificate means handling its private key, which §2.1 already treats
as part of the attack surface, not a convenience to add to. More tools
will follow as they are discovered, and each earns its place the way §0
already asks: by working on what is committed to a repository, not on
the machine at large.

Alongside the tool slot, every pane - Folders, Contents and the tool
slot alike - can pop out into its own window inside the same
application and dock back. GUIDANCE.md §2.6 writes down the mechanics:
one shared selection with a pin, what closing a window does, the
last-window rule, and remembered layout.

### Why

The owner wants the editor, certificate management and whatever tool
comes after them to stand on their own: a reader working through a
certificate audit across forty repositories should not have to keep the
Folders tree in the same rectangle as the thing they are reading. A
fixed three-column window has no room for that; a tool slot that can
float does.

### What already supports it

§2's service/thin-front-end split means a second window costs nothing
structural: every window is another view on the same `App`, talking to
the one connection to the service, not a second copy of the
application. The plugin split in §3.1 already separates a tool's core
from its presentation, so a certificate tool is one more crate in that
shape, not a new kind of thing. Nothing about the inter-process
communication (IPC) boundary, the security model in §2.1, or D10's
read-only stance on source control changes to accommodate this - the
new surface is windows and where tools live, not what any of them is
allowed to touch.

### What it would take to revisit

The tool slot and the pop-out mechanism are each one seam. Which tool is
shown for a selection is a lookup the same shape as the file-plugin
dispatch in §3.1; a window's docked-or-floating state is one property
per pane, read at startup and written at shutdown alongside the rest of
window layout. Reverting to a fixed third pane means removing the
pop-out affordance from that one property, which is smaller than adding
it was. Reverting the tool slot to "the File pane" means taking the
certificate tool back out of scope, which only a decision superseding
this one can do.

### What this supersedes

GUIDANCE.md §2.4 as it read before this work order, which named three
panes and called the third "the File pane, supplied entirely by the
file-type plugin". GUIDANCE.md §3.6 "The File pane is an editor" (D14)
is renamed by this work order to "The editor tool"; D14's editing
boundary is unchanged and still binding - this only renames what hosts
the editor, not what the editor may do.
