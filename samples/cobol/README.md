# csvstats

Summary statistics for a file of readings, in COBOL.

    cobc -x -free csvstats.cbl -o csvstats
    ./csvstats

`REPORT-DEVIATION` is never performed, and the paragraph above it ends
with `STOP RUN`, so nothing reaches it. It is left that way so the file
pane has something to warn about.
