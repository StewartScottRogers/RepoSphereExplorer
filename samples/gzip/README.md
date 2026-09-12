# readings

`readings.csv.gz` carries an original file name, a modification time and
a comment in its header — all readable without decompressing anything.
Python's own `gzip` writes no comment, so this member was written by
hand.

`readings.tar.gz` is `samples/tar/readings.tar` compressed. It is here so
the tar plugin can be seen **not** to claim it: tar's magic sits at
offset 257 of the plain bytes, and there are no plain bytes here.
