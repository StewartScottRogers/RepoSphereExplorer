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
no. Working-tree status is deferred with the operations, because reporting
whether a clone is dirty needs the same walk of the work tree that they
need; the field exists and reports unknown until then.

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
