CC      ?= cc
CFLAGS  ?= -std=c17 -Wall -Wextra -Wpedantic -Wshadow -Wconversion -Werror -O2
CPPFLAGS = -Iinclude

BUILD := build
BIN   := $(BUILD)/ringbuffer

.PHONY: all test clean

all: $(BIN)

$(BIN): src/ringbuffer.c include/ringbuffer.h | $(BUILD)
	$(CC) $(CFLAGS) $(CPPFLAGS) -o $@ src/ringbuffer.c

$(BUILD):
	mkdir -p $(BUILD)

test: $(BIN)
	./$(BIN)

clean:
	rm -rf $(BUILD)
