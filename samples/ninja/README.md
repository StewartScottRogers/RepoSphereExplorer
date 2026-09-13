# build.ninja

Ninja is not written by people. It is what a build system emits, and it
is read by people exactly once - when something has gone wrong and they
need to know what the generator actually asked for.

So what matters is the mapping: which rule builds a given output, from
which inputs, with which command. This file declares four rules, builds
eight outputs, has two `phony` targets that exist only to be named on
the command line, one `default`, and pulls in two more files - an
`include`, which shares this file's variables, and a `subninja`, which
does not.
