# readings

`readings.csv.zst` declares its content size in the frame header and
carries a checksum, so the pane can report a ratio from the header alone.

`needs-a-dictionary.zst` was compressed against `readings.dict` and will
not decompress without it. The dictionary identifier in the frame header
is the only thing saying which dictionary — that is what the pane
reports, because a frame arriving without its dictionary is otherwise
indistinguishable from a corrupt one.
