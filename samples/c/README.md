# ringbuffer

A lock-free single-producer, single-consumer ring buffer of fixed-size
messages.

## Using it

```c
struct ring_buffer ring;
ring_init(&ring);

ring_push(&ring, 1, "first");

struct ring_slot slot;
while (ring_pop(&ring, &slot)) {
    handle(&slot);
}

ring_stats stats = ring_drain(&ring);
printf("%zu drained, %lu dropped\n", stats.drained, stats.dropped);
```

## Notes

- A full buffer **drops** and counts the drop; it does not overwrite. A
  buffer that silently discards the oldest message is a buffer that loses
  the beginning of every incident.
- One producer and one consumer, no lock. Two producers need one, and this
  does not pretend otherwise.
- Capacity and message size are compile-time constants, so the whole thing
  is one allocation the caller already owns.

## Building

```bash
make test
# or
cmake -B build && cmake --build build && ctest --test-dir build
```

---

**This is a fixture.** It lives in `samples/c/` so the application has a
C project to open, not just a C file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
