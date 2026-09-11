# csvstats

Summary statistics for a comma-separated file, in Zig.

    zig build
    zig build run -- data.csv
    zig build test

`src/root.zig` is the library and `src/main.zig` the command. The private
`describe` function in the library takes a buffer from the allocator and
never gives it back; it is left that way so the file pane has something
to warn about.
