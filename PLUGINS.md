# Plugin registry

Tracks every file-type plugin this project has built or rejected, per
[GUIDANCE.md §3.1](GUIDANCE.md#31-plugin-architecture) and
[§7 D5](GUIDANCE.md#7-open-decisions). Consulted before proposing or
building any plugin work order, so the factory never duplicates a plugin
that already exists or retries a format already ruled out.

## Built

| Format | Crate | Issue | Notes |
| --- | --- | --- | --- |
| Plain text | `crates/plugins/text` | — | First plugin, built with the architecture itself |
| Image (raster codecs) | `crates/plugins/image` | — | |
| Archive (zip) | `crates/plugins/archive` | — | Also provides `extract` for the D4 operations |
| PDF | `crates/plugins/pdf` | — | |
| Directory-as-file | `crates/plugins/directory` | — | |
| Python source | `crates/plugins/python` | #7 | Sniffs by shebang or top-level `def`/`class`, ahead of `text` in `CORE_PLUGINS` |

## Rejected / infeasible

_(none yet)_

## Maintaining this file

- **Built**: when a plugin's PR merges, add a row here with its format,
  crate path, and issue number, in the same PR or a prompt follow-up.
- **Rejected**: when a work order concludes a format can't reasonably
  become a plugin, add a row here with the format, its issue number, and
  a one-line reason, then close the issue without a PR. Do not reopen or
  retry a rejected format's work order unless explicitly asked to.
- Before proposing or starting any plugin work order, check both
  sections here, plus `gh issue list --label work-order --state all`
  for one already filed on the same format.
