# csvstats

Summary statistics for a comma-separated file, in D.

    dub build
    dub run -- data.csv
    dub test

`source/csvstats.d` is the library and `source/app.d` the command.
`Reader.read` touches the filesystem and carries no safety attribute, so
it is `@system` by default; it is left that way so the file pane has
something to warn about.
