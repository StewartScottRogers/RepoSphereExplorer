// Command worker runs the pool against a fixed number of jobs and prints
// what came back, so the package has a front door you can actually run.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"time"

	pool "github.com/example/worker-pool"
)

func main() {
	workers := flag.Int("workers", 4, "how many workers to run")
	jobs := flag.Int("jobs", 20, "how many jobs to submit")
	timeout := flag.Duration("timeout", 5*time.Second, "how long to wait for all of them")
	flag.Parse()

	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()

	p := pool.New(*workers)
	results, err := p.Run(ctx, *jobs, func(ctx context.Context, n int) (int, error) {
		return n * n, nil
	})
	if err != nil {
		fmt.Fprintf(os.Stderr, "worker: %v\n", err)
		os.Exit(1)
	}

	var total int
	for _, r := range results {
		total += r
	}
	fmt.Printf("%d results, total %d\n", len(results), total)
}
