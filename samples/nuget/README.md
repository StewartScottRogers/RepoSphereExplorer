# CsvStats.1.0.3.nupkg

A NuGet package: a zip built to the Open Packaging Conventions, so it
carries a `[Content_Types].xml` and a `_rels/.rels` beside the
`.nuspec` that actually describes it.

Two target framework groups, with different dependencies in each -
which is the point of grouping them. It also ships a build targets
file, so installing it changes how the consuming project builds, and
that is worth telling a reader.
