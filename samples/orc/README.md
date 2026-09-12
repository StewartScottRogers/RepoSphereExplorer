# readings.orc

Twenty thousand sensor readings in the Optimized Row Columnar (ORC)
format, written with zlib compression.

Seven columns: an identifier, a string, a double, a boolean, a
timestamp, a struct and a list of strings. The last two are there
because a nested type is where a reader of this format usually stops
being able to say anything useful.

**One stripe, not several.** ORC closes a stripe when it has buffered
tens of megabytes, and no setting persuades the writer otherwise for
data this size — twenty thousand rows of this compress to about twelve
kilobytes. A fixture with several stripes would have to be megabytes,
which is not worth it to prove a counter. The plugin reads and reports
the stripe count either way.
