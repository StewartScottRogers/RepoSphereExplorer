# tree-indexer

A bounded, cancellable directory indexer: walks a tree on a worker thread,
reports progress as it goes, and stops the moment it is asked to.

## Using it

```rust
use tree_indexer::index;

let entries = index(std::path::Path::new("."))?;
println!("{} entries", entries.len());
```

Or from the command line:

```bash
cargo run --release -- /some/directory
```

## Limits

- The walk refuses to descend past `MAX_DEPTH` (64). A symlink loop is the
  reason that limit exists.
- Progress is reported every 256 entries, not per entry: a report per file
  costs more than the walk does.

## Licence

MIT.

---

**This is a fixture.** It lives in `samples/rust/` so the application has a
Rust project to open, not just a Rust file. It is valid and internally
consistent, but no pipeline builds it — see `samples/README.md`.
