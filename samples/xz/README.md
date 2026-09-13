# readings

`readings.csv.xz` uses the default LZMA2 filter alone with a CRC-64
check. `filtered.xz` puts a delta filter in front of LZMA2 and checks
with SHA-256 — the shape a writer uses for numbers that change a little
at a time.

Both carry an index at the end giving the uncompressed size, which is why
the pane can report a ratio without decompressing anything.
