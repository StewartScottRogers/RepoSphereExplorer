# readings

`readings.csv.bz2` is twelve thousand rows compressed at level 1, which
closes a block every hundred thousand bytes — so the file has several
blocks rather than one.

`two-streams.bz2` is two whole bzip2 streams written one after another,
which is what `cat a.bz2 b.bz2` leaves behind. A reader that stops at the
first end-of-stream marker reports half the file, so the fixture is here
to catch that.

bzip2 records no uncompressed size anywhere. The pane says so rather than
guessing.
