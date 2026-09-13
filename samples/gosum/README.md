# go.sum

Not a lock file, though it is often called one. `go.mod` decides the
versions; this records what the contents of those versions hashed to
the first time anybody fetched them, so a later fetch that differs is
caught.

Two lines per module: one for the module's own content, one for its
`go.mod` alone - Go needs the second to work out the version graph
without downloading everything.

Three things in here a reader meets: `go-cmp` appears at two versions,
because something in the graph still asks for the older one; the
`xeipuuv` module has a pseudo-version, built from a date and a commit
because it has no tags; and `client-go` carries `+incompatible`,
meaning a major version past one that never adopted modules.
