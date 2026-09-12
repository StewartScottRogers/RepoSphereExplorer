# csvstats.el

Summary statistics for a selected region of comma-separated numbers.

    M-x csvstats-mean
    M-x csvstats-deviation

`csvstats-deviation` has no `;;;###autoload` cookie, so it cannot be run
by name until something else has loaded the file.
`csvstats-compat.el` has no `lexical-binding` header, so every `let` in
it is dynamically bound. Both are left that way so the file pane has
something to warn about.
