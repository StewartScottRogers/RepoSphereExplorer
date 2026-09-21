module github.com/example/pipeline

go 1.24

toolchain go1.24.1

require (
	github.com/google/uuid v1.6.0
	github.com/spf13/cobra v1.8.1
	github.com/prometheus/client_golang v1.20.2
)

require github.com/inconshreveable/mousetrap v1.1.0 // indirect

exclude github.com/legacy/broken v0.9.0

replace github.com/example/pipeline/internal/shared => ../shared

replace github.com/upstream/thing => github.com/example/thing-fork v1.2.3

retract v0.1.0 // published by mistake, before the module had a public API

godebug (
	http2client=0
)
