---
title: Writing a plugin
description: What a Markdown fixture has to carry.
tags: [plugins, guide]
---

# Writing a plugin

A plugin is one crate with two halves. This document exists to exercise
every field the Markdown plugin extracts, so it carries a heading outline,
fenced code in two languages, links, an image, a task list, a table and
front matter.

## The core half

The core half is linked into the service and never touches a toolkit.

```rust
impl PluginCore for GuideCore {
    fn name(&self) -> &'static str {
        "guide"
    }
}
```

## The presentation half

The presentation half is linked into both front ends and never touches raw
bytes.

```sh
cargo test -p plugin-guide
```

### What to check

| check | why |
| --- | --- |
| both halves agree on extensions | or a listing marks a file its viewer refuses |
| the fixture fills every field | or `sample_coverage.rs` fails |

## Before you open the pull request

- [x] the three gates pass
- [x] the fixture exercises every field
- [ ] `PLUGINS.md` has its row
- [ ] the work order's acceptance checks are ticked

See [the guidance](../../GUIDANCE.md) and [the decisions](../../DECISIONS.md).

![The plugin's two halves](../image/logo.png)

Setext still counts
-------------------

And a level-one setext heading does too.
