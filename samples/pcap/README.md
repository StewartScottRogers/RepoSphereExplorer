# readings.pcap and readings.pcapng

The same four packets in both capture formats.

`readings.pcap` is the classic format: a twenty-four byte header, then
one length-prefixed record per packet. It carries a link type and a
snapshot length and nothing else - no interface names, no comments,
nowhere to say which interface a packet arrived on.

`readings.pcapng` is what replaced it, and the difference is the point:
a section header block naming the machine and the tool, two interface
description blocks with names and descriptions, and a per-packet comment
on the request that timed out. Each packet says which interface it came
from, which the classic format cannot.

The packets are real Ethernet frames carrying IPv4 and UDP, checksums
included: two DNS lookups and two of an HTTP exchange over UDP.
