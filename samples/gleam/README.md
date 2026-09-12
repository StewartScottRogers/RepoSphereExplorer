# csvstats

Summary statistics for a column of readings, in Gleam.

    gleam build
    gleam test

`describe` and `parse_line` are public and neither signature says what
comes back, so a caller has to read the body to find out. Both are left
that way so the file pane has something to warn about, along with the
`Handle` type, which is opaque, and `square_root`, whose body is Erlang
on one target and JavaScript on the other.
