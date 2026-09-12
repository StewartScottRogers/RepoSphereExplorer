# pickles

Three pickles of the same readings, at protocols 2, 4 and 5: a mapping,
a sequence, and an instance of a class.

**Nothing here is ever unpickled.** The plugin reads the opcode stream
and never runs it, because unpickling untrusted data executes whatever
the stream says to. `instance-protocol-5.pickle` refers to a class by
name — `__main__.Station` — which is exactly what a reader needs warning
about: loading it would import that module.
