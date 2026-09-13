# uv.lock and poetry.lock

The same problem solved by two tools, and they do not write the same
file.

`uv.lock` records a resolution across *several* environments at once:
`resolution-markers` at the top, a `marker` on individual dependencies,
and every wheel and source distribution with its hash and size. A
package can come from the registry, from a git revision, or - for the
project itself - be editable.

`poetry.lock` records one resolution, with `optional` and
`python-versions` per package, the files as a flat list, and a
`content-hash` in `[metadata]` that ties it to the `pyproject.toml`
beside it.

Both are TOML with `[[package]]` tables, which is why telling which
tool wrote one is the first thing a reader wants.
