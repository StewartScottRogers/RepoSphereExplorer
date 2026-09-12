# csvstats.awk

Summary statistics for a comma-separated file.

    awk -f csvstats.awk samples.csv

`carried` inside `average()` is not one of its parameters, so it is
global and survives the call — awk spells a local variable as an extra
parameter nobody passes. It is left that way so the file pane has
something to warn about.
