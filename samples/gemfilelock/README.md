# Gemfile.lock

What Bundler resolved, which is not what the Gemfile asked for.

The sections are the point. `GEM` is what came from rubygems.org;
`PATH` is a gem vendored into the repository; `GIT` is one pinned to a
commit. A reader chasing "where did this version come from" needs those
told apart.

`DEPENDENCIES` is the direct list - the ones the Gemfile actually named
- and everything under `specs:` that is not in it arrived transitively.
The `!` suffix marks a dependency that came from somewhere other than
the default source.
