# BUILD.bazel and readings.bzl

A Bazel package and the rule it loads.

`BUILD.bazel` is Starlark used declaratively: `load` statements bringing
in rules from other repositories, then calls to those rules. It
instantiates six targets across five kinds, globs its sources - one of
them with an `exclude` - and sets visibility three different ways: a
package default of private, `//visibility:public` on what is meant to be
used elsewhere, and a single named package on the documentation.

`readings.bzl` is Starlark used as a language: a rule implementation, an
action, and a `rule()` call declaring the attributes it takes. The
leading underscore on `_generator` marks it private to the rule, which
is a convention the language enforces nowhere and everybody follows.
