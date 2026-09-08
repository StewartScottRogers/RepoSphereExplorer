/*
 * A fixed-capacity ring buffer, the sort of thing that turns up in an
 * embedded logging path: no allocation, one writer, one reader.
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>

#define RING_CAPACITY 8

struct ring_slot {
    unsigned long sequence;
    char payload[32];
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

/* "struct " inside a string literal, which a line-oriented extractor
   should not mistake for a declaration. */
static const char *kBanner = "struct ring_buffer, capacity 8";

void ring_init(struct ring_buffer *ring)
{
    memset(ring, 0, sizeof(*ring));
}

static size_t ring_next(size_t index)
{
    return (index + 1) % RING_CAPACITY;
}

bool ring_push(struct ring_buffer *ring, unsigned long sequence, const char *text)
{
    size_t next = ring_next(ring->head);
    if (next == ring->tail) {
        ring->dropped++;
        return false;
    }
    struct ring_slot *slot = &ring->slots[ring->head];
    slot->sequence = sequence;
    snprintf(slot->payload, sizeof(slot->payload), "%s", text);
    ring->head = next;
    return true;
}

bool ring_pop(struct ring_buffer *ring, struct ring_slot *out)
{
    if (ring->head == ring->tail) {
        return false;
    }
    *out = ring->slots[ring->tail];
    ring->tail = ring_next(ring->tail);
    return true;
}

ring_stats ring_drain(struct ring_buffer *ring)
{
    ring_stats stats = {0, ring->dropped};
    struct ring_slot slot;
    while (ring_pop(ring, &slot)) {
        printf("[%lu] %s\n", slot.sequence, slot.payload);
        stats.written++;
    }
    return stats;
}

int main(void)
{
    struct ring_buffer ring;
    ring_init(&ring);
    puts(kBanner);

    for (unsigned long i = 0; i < 12; i++) {
        char line[32];
        snprintf(line, sizeof(line), "message %lu", i);
        ring_push(&ring, i, line);
    }

    ring_stats stats = ring_drain(&ring);
    printf("wrote %zu, dropped %zu\n", stats.written, stats.dropped);
    return 0;
}
