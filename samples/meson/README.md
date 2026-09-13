# meson.build

The same build as `samples/cmake/`, in the other build system, so the
two can be read against each other.

Meson says the same things in a different shape: `project()` takes the
version and languages as keyword arguments, `dependency()` finds a
package and can be told a version range, and `get_option()` reads a
knob rather than declaring one - the declaring happens in a
`meson_options.txt` elsewhere.

The `required : with_zlib` on the third dependency is worth noticing: a
dependency that is only looked for when an option says so, which is how
Meson writes what CMake writes with an `if`.
