# `samples/`

One directory per file-type plugin, holding files that plugin owns. Two
audiences: a person checking the application by opening things, and the
test suite.

Three tests hold this set to its job:

- `service`'s `samples.rs` — every file is recognised by the plugin whose
  directory it sits in, and by no other.
- `service`'s `sample_coverage.rs` — every list a plugin puts on the wire
  has something in it, and every optional field is filled, unless the
  format cannot carry it. Those exceptions are named in `ALLOWED_GAPS`,
  one line each, with the reason.
- `gui`'s `file_pane.rs` — every fixture renders lines in the File pane,
  and every picture a plugin offers is well formed.

## What the fixtures are

They are working files, not keyword salad: programs that would compile,
documents that would open, a font that would install, media that would
play. A fixture that only proves a parser did not crash is a fixture that
will one day hide a bug, because everything passes when nothing is asked.

That is not hypothetical here. Rewriting this set found four defects the
old one had been hiding for its whole life:

- a C file with a top-level `struct` opened as Rust, and every Java file
  opened as Perl (#272)
- a modern TypeScript module reported no classes, no interfaces and no
  functions, and four other extractors were as blind (#274)
- a font whose name table is ordered the way the specification requires
  reported no family (#274)
- an AVI reported no codec, though its stream header names one (#274)

Every one of them needed a file with something in it.

## A few worth knowing about

| fixture | why it is there |
| --- | --- |
| `text/access.log` | 96 KiB, past the 64 KiB read cap, so `truncated` is exercised by a real file. It is the only fixture that is deliberately long. |
| `certificate/chain.pem` | A leaf certificate, the root that signed it, and the leaf's key. The key protects nothing and is safe to publish. |
| `directory/` | The `directory` plugin has no file to sniff: the folder itself is what it recognises, so this one holds ordinary files of assorted types. |
| `model3d/` | Two fixtures, because OBJ and glTF carry different halves of the view: geometry counts from one, scene and generator from the other. |
| `image/`, `audio/`, `video/` | Generated, and genuinely playable and viewable. Open them. |

## Binary fixtures and line endings

The binary fixtures are generated rather than downloaded, so the set
carries no third-party licences and every byte is accounted for.

They are marked `binary` in the repository's `.gitattributes`. This is
not decoration: a PDF's cross-reference table is a list of byte offsets,
and a checkout that rewrote one newline inside it produced a file that no
longer parsed — on Windows only, and never in CI.

## Adding a fixture

Per rule 9 in `CLAUDE.md`, a new plugin's work order adds its
`samples/<name>/` entry in the same PR. Make it a real file of that type,
and make it drive every field the plugin extracts — `sample_coverage.rs`
will tell you if it does not.
