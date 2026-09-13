# column.cpython-311.pyc and column.cpython-314.pyc

Python bytecode from two CPython releases, and the source both were
compiled from.

The two headers are not the same shape. The 3.11 file is
timestamp-based: it records when `column.py` was last written and how
long it was, and the interpreter recompiles if either has changed. The
3.14 file is hash-based: it records a hash of the source instead, which
is what makes a build reproducible. Reading a `.pyc` means telling those
two apart, because the same four header bytes mean different things.
