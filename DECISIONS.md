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
clone is dirty needs the same walk of the work tree. That was true of a
single pass over every checkout at once and not of a row asked for on its
own: read from the checkout's own index, one repository at a time, after
the listing itself is drawn, the answer costs a pass over its tracked files
and nothing more. Status is now reported for every row on screen, and in
the tool slot for the selected one. What D10 still settles is unchanged -
this application reads and never drives, and clone, fetch, pull and commit
remain out of scope.

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
GUIDANCE.md §3.5.

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

The right-hand pane stops being "the File pane" and becomes a **tool
slot**: the tool that applies to the selection is shown, a picker appears
when more than one applies, and the editor (§3.6) is its first tool. A
certificate tool - read-only, finding the certificates committed under the
Repos Directory and saying which are expired or expiring - is its second.
More tools follow as they are discovered.

Every pane - Folders, Contents and the tool slot alike - can pop out into
its own window and dock back, inside the same application: one process,
one connection to the service, several windows, never a separate program.
Windows are linked by default, following one shared selection; a
popped-out tool window can be pinned to keep what it shows while the
selection moves on. Closing a popped-out window docks its pane back; the
application exits when its last window closes. Which panes are popped
out, and each window's position and size, are remembered between
launches - window arrangement, not location, so every launch still opens
at the Repos Directory regardless (D7 is unchanged). Tools are compiled
in, like plugins (§3.2); there is no loadable third-party tool. The
terminal front end is unaffected: this is a graphical front end decision.

**Why.** The owner wants the editor, certificate management and whatever
tool comes after them to be things that stand on their own - inspectable,
comparable side by side, movable to a second monitor - rather than
permanently wedged into one third of one window. A certificate tool needs
exactly the working-copy detection this application already does (D7,
D10) and nothing it has not already been asked to add.

**What already supports it.** The service / thin front end split in §2
means a second window is another view onto the same `App`, not a second
copy of anything: the pane behind each window already goes through the
same service connection. §3's plugin trait pair - a core half and a
presentation half - is the shape a tool trait pair also takes, so the
tool slot is a small extension of a pattern already built, not a new one.

**What it costs.** Multi-window layout: remembering and restoring several
windows' position and size, and keeping one shared selection consistent
across however many are open. The certificate tool adds a read of files
this application did not previously look inside - certificates - though
never their private keys, and never to issue, renew, revoke or deploy one
(§2.1, §6).

**What it would take to revisit.** The tool slot is one concept - a
`Tool` trait pair and a registry, the same shape §3.2 already uses for
plugins - so adding a third tool means implementing that pair, not
touching window or pane code. Undoing the docking/floating decision means
returning the tool slot to a fixed third column and removing the
per-window remembered layout; the selection stays shared either way since
nothing else depends on windows being separate.

**What this supersedes.** GUIDANCE.md §2.4's old "three panes" wording,
which named the third pane "the File pane" and left it fixed in the main
window. §3.6's old title, "The File pane is an editor", which named the
pane rather than the tool now living in its slot.
