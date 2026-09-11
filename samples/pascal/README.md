# csvstats

Summary statistics for a comma-separated file, in Object Pascal.

    fpc -Mobjfpc report.pas
    ./report

`csvstats.pas` declares `WriteReport` in its interface and never writes a
body for it, so the unit will not link. It is left that way so the file
pane has something to warn about.
