module github.com/example/greeter

go 1.22

toolchain go1.22.3

require (
	github.com/spf13/cobra v1.8.1
	github.com/google/uuid v1.6.0
	golang.org/x/sync v0.7.0 // indirect
)

replace github.com/spf13/cobra => github.com/example/cobra-fork v1.8.1-patched
