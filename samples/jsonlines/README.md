# readings

`readings.jsonl` is 240 sensor readings, one JSON value per line. Every
record carries `id`, `station`, `reading` and `taken_at`; one in nine
also carries `calibration_offset`, and a few carry `flagged_by`. That
raggedness is the point: anything loading the file as a table has to
decide what to do about the records that lack those keys.

`interrupted.jsonl` is the same thing with a line cut off part-way, as
happens when a writer is killed mid-record. It is there so the file pane
has something to warn about.
