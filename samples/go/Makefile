GO ?= go
PKG := ./...

.PHONY: all build test vet fmt clean

all: fmt vet test

build:
	$(GO) build -o build/pipeline ./cmd/pipeline

test:
	$(GO) test -race -count=1 $(PKG)

vet:
	$(GO) vet $(PKG)

fmt:
	$(GO) fmt $(PKG)

clean:
	rm -rf build
