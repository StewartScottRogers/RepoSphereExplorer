# csvstats

Summary statistics for a comma-separated file: the mean and standard
deviation of every column that holds numbers.

    luarocks make csvstats-1.0-1.rockspec
    lua tools/report.lua data.csv

Run the tests with [busted](https://lunarmodules.github.io/busted/):

    busted

`tools/report.lua` is deliberately written with global names, as a
one-off script would be. It is there so the file pane has something to
warn about.
