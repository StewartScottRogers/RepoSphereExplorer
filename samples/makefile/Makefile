# Build, test and package a small C project, with the pattern rules,
# variables and phony targets a real Makefile carries.

CC        ?= cc
AR        ?= ar
PREFIX    ?= /usr/local
BUILD_DIR ?= build
VERSION   := $(shell git describe --tags --always --dirty 2>/dev/null || echo 0.0.0)

WARNINGS  := -Wall -Wextra -Wpedantic -Wshadow -Wconversion
CFLAGS    := -std=c17 -O2 -g $(WARNINGS) -DVERSION=\"$(VERSION)\"
LDFLAGS   :=
LDLIBS    := -lm

SOURCES   := $(wildcard src/*.c)
OBJECTS   := $(patsubst src/%.c,$(BUILD_DIR)/%.o,$(SOURCES))
DEPS      := $(OBJECTS:.o=.d)
TEST_SRC  := $(wildcard tests/*.c)
TEST_BIN  := $(patsubst tests/%.c,$(BUILD_DIR)/test-%,$(TEST_SRC))

TARGET    := $(BUILD_DIR)/ringbuffer
LIBRARY   := $(BUILD_DIR)/libringbuffer.a

.PHONY: all clean test install uninstall fmt lint dist help
.DEFAULT_GOAL := all

all: $(TARGET) $(LIBRARY) ## Build the binary and the static library

$(BUILD_DIR):
	@mkdir -p $@

$(BUILD_DIR)/%.o: src/%.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -MMD -MP -c $< -o $@

$(TARGET): $(OBJECTS)
	$(CC) $(LDFLAGS) $^ $(LDLIBS) -o $@

$(LIBRARY): $(filter-out $(BUILD_DIR)/main.o,$(OBJECTS))
	$(AR) rcs $@ $^

$(BUILD_DIR)/test-%: tests/%.c $(LIBRARY) | $(BUILD_DIR)
	$(CC) $(CFLAGS) -Isrc $< $(LIBRARY) $(LDLIBS) -o $@

test: $(TEST_BIN) ## Build and run every test binary
	@failed=0; \
	for binary in $(TEST_BIN); do \
		printf '%-28s' "$$(basename $$binary)"; \
		if $$binary >/dev/null 2>&1; then echo ok; else echo FAILED; failed=1; fi; \
	done; \
	exit $$failed

fmt: ## Format sources in place
	clang-format -i $(SOURCES) $(TEST_SRC) $(wildcard src/*.h)

lint: ## Static analysis, warnings as errors
	$(CC) $(CFLAGS) -fsyntax-only -Werror $(SOURCES)

install: all ## Install to $(PREFIX)
	install -d $(DESTDIR)$(PREFIX)/bin $(DESTDIR)$(PREFIX)/lib
	install -m 0755 $(TARGET) $(DESTDIR)$(PREFIX)/bin/
	install -m 0644 $(LIBRARY) $(DESTDIR)$(PREFIX)/lib/

uninstall: ## Remove what install put down
	$(RM) $(DESTDIR)$(PREFIX)/bin/$(notdir $(TARGET))
	$(RM) $(DESTDIR)$(PREFIX)/lib/$(notdir $(LIBRARY))

dist: all ## Make a release tarball
	@tar -czf $(BUILD_DIR)/ringbuffer-$(VERSION).tar.gz -C $(BUILD_DIR) \
		$(notdir $(TARGET)) $(notdir $(LIBRARY))
	@echo "wrote $(BUILD_DIR)/ringbuffer-$(VERSION).tar.gz"

clean: ## Remove every build artefact
	$(RM) -r $(BUILD_DIR)

help: ## List the targets
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  %-12s %s\n", $$1, $$2}'

-include $(DEPS)
