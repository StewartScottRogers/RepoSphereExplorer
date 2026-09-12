# readings.cbor

A Concise Binary Object Representation (CBOR) map of sensor readings.

It opens with the self-describing tag `d9 d9 f7`, so a reader meeting
the bytes cold knows what they are. Inside: nested arrays, a byte string,
tag 1 (a point in time) and tag 32 (a uniform resource identifier), and
two indefinite-length items — an array and a text string written in
pieces. Those last are what a streaming writer emits when it does not yet
know how long the value will be, and they are the awkward part of the
format, so the fixture has them.
