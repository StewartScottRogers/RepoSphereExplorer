// Command pipeline runs the package over a handful of jobs and prints what
// came back, so the package has a front door you can actually run.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/example/pipeline"
)

// shoutHandler is the handler the command uses: it upper-cases a job's
// payload, and refuses an empty one rather than returning empty output.
type shoutHandler struct{}

func (shoutHandler) Handle(ctx context.Context, job pipeline.Job) (string, error) {
	if strings.TrimSpace(job.Payload) == "" {
		return "", fmt.Errorf("job %d: empty payload", job.ID)
	}
	select {
	case <-ctx.Done():
		return "", ctx.Err()
	default:
		return strings.ToUpper(job.Payload), nil
	}
}

func main() {
	workers := flag.Int("workers", 4, "how many workers to run")
	timeout := flag.Duration("timeout", 5*time.Second, "how long to give the whole run")
	flag.Parse()

	payloads := flag.Args()
	if len(payloads) == 0 {
		fmt.Fprintln(os.Stderr, "usage: pipeline [-workers n] payload...")
		os.Exit(2)
	}

	jobs := make([]pipeline.Job, 0, len(payloads))
	for i, payload := range payloads {
		jobs = append(jobs, pipeline.Job{ID: i + 1, Payload: payload, Timeout: *timeout})
	}

	ctx, cancel := context.WithTimeout(context.Background(), *timeout)
	defer cancel()

	results, err := pipeline.Run(ctx, *workers, jobs, shoutHandler{})
	if err != nil {
		fmt.Fprintf(os.Stderr, "pipeline: %v\n", err)
		os.Exit(1)
	}

	for _, result := range results {
		if result.Err != nil {
			fmt.Fprintf(os.Stderr, "job %d: %v\n", result.JobID, result.Err)
			continue
		}
		fmt.Println(result.Output)
	}

	stats := pipeline.Summarise(results)
	fmt.Fprintf(os.Stderr, "%d completed, %d failed, slowest %s\n",
		stats.Completed, stats.Failed, stats.Slowest)
}
