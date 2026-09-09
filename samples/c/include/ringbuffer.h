/* A lock-free single-producer, single-consumer ring buffer.
 *
 * One producer calls ring_push, one consumer calls ring_pop, and neither
 * takes a lock. Two producers would need one, and this deliberately does
 * not provide it: a buffer that is only safe under a condition nobody
 * stated is worse than one that says what it is.
 */

#ifndef RINGBUFFER_H
#define RINGBUFFER_H

#include <stdbool.h>
#include <stddef.h>

#define RING_CAPACITY 8
#define RING_PAYLOAD_MAX 32

struct ring_slot {
    unsigned long sequence;
    char payload[RING_PAYLOAD_MAX];
};

struct ring_buffer {
    struct ring_slot slots[RING_CAPACITY];
    size_t head;
    size_t tail;
    size_t dropped;
};

typedef struct {
    size_t written;
    size_t dropped;
} ring_stats;

/* Puts the buffer into its empty state. Safe to call on a buffer holding
   anything at all; nothing is freed, because nothing was allocated. */
void ring_init(struct ring_buffer *ring);

/* Appends one message. Returns false and counts a drop when the buffer is
   full, rather than overwriting what the consumer has not read. */
bool ring_push(struct ring_buffer *ring, unsigned long sequence, const char *text);

/* Takes the oldest message. Returns false when there is nothing to take. */
bool ring_pop(struct ring_buffer *ring, struct ring_slot *out);

/* Empties the buffer, reporting how much came out and how much had been
   dropped since the last drain. */
ring_stats ring_drain(struct ring_buffer *ring);

#endif /* RINGBUFFER_H */
