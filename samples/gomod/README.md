# go.mod

The module path a checkout publishes as, the Go version and toolchain it
builds with, and what it depends on.

Three things in here a reader meets: one requirement is marked
`// indirect`, needed by a dependency rather than this module's own code;
one `replace` points at a sibling directory (`../shared`), the sign that
this checkout is one leg of a larger repository; and a second `replace`
points at a fork instead, `github.com/example/thing-fork`, at its own
pinned version. An `exclude`, a `retract` with its reason, and a
`godebug` setting round out the directives this plugin reads.
