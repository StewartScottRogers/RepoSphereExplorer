# CsvStats.dll, CsvStats.Cli.dll and fr/CsvStats.resources.dll

Three real assemblies, compiled by the .NET SDK from `Column.cs`, a
console program beside it, and `Strings.fr.resx`, and kept here as the
compiler emitted them.

They differ in the ways that matter to a reader:

- **`CsvStats.dll`** is a library. It has no entry point — something
  else calls into it — and it is **strong-named**, signed with a key so
  the runtime can tell it from another assembly claiming the same name.
  It defines four public types: an enum, an interface, a record struct
  and a class.
- **`CsvStats.Cli.dll`** is the program. It names the method the runtime
  starts at, it is *not* strong-named, and it has no public types at all
  — a console program's `Program` class is internal, which is what makes
  "no public types" a real answer rather than a failure to find any.

- **`fr/CsvStats.resources.dll`** is a satellite: the French
  translations, in a separate assembly under a directory named for its
  culture. It is the only one of the three with a **culture** — the
  assembly it belongs to is culture-neutral, and the pair is what makes
  that field mean anything. It holds no types at all, only resources.

All were built with `Deterministic`, so recompiling the same source
produces the same bytes.

`Column.cs` is here because an assembly is unreadable without it: the
assembly has the shape — the tables, the tokens, the flags — and the
source says what that shape meant.
