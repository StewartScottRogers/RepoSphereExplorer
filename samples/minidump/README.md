# csvstats.dmp

A Windows minidump: a header, a directory saying where each stream
landed, and the streams themselves.

This one carries the five streams a reader actually wants - the thread
list, the module list, the exception record, the system information and
the process information - and no memory, which is what a small crash
dump looks like. The exception is an access violation reading address
`0x10`, on the thread that faulted, which is a null pointer plus an
offset and the most common crash there is.
