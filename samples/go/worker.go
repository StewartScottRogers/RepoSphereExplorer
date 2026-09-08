// Package pipeline runs a bounded pool of workers over a job channel and
// collects their results, cancelling everything if any one of them fails.
package pipeline

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"sync"
	"time"
)

// Job is one unit of work handed to a worker.
type Job struct {
	ID      int
	Payload string
	Timeout time.Duration
}

// Result is what a worker reports back for a Job.
type Result struct {
	JobID    int
	Output   string
	Duration time.Duration
	Err      error
}

// Stats summarises a completed run.
type Stats struct {
	Completed int
	Failed    int
	Slowest   time.Duration
}

// Handler turns a Job into its output. A handler must respect ctx.
type Handler interface {
	Handle(ctx context.Context, job Job) (string, error)
}

// Reporter is told about each result as it lands.
type Reporter interface {
	Report(result Result)
}

// ErrNoWorkers is returned when a pool is asked to run with no workers.
var ErrNoWorkers = errors.New("pipeline: worker count must be positive")

type upperHandler struct{}

func (upperHandler) Handle(ctx context.Context, job Job) (string, error) {
	select {
	case <-ctx.Done():
		return "", ctx.Err()
	case <-time.After(time.Millisecond):
		if job.Payload == "" {
			return "", fmt.Errorf("job %d: empty payload", job.ID)
		}
		return fmt.Sprintf("handled %s", job.Payload), nil
	}
}

// Run fans jobs out to workers, returning every result in job order.
func Run(ctx context.Context, workers int, jobs []Job, handler Handler) ([]Result, error) {
	if workers <= 0 {
		return nil, ErrNoWorkers
	}

	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	queue := make(chan Job)
	results := make(chan Result, len(jobs))

	var wg sync.WaitGroup
	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for job := range queue {
				started := time.Now()
				output, err := handler.Handle(ctx, job)
				results <- Result{
					JobID:    job.ID,
					Output:   output,
					Duration: time.Since(started),
					Err:      err,
				}
			}
		}()
	}

	go func() {
		defer close(queue)
		for _, job := range jobs {
			select {
			case <-ctx.Done():
				return
			case queue <- job:
			}
		}
	}()

	wg.Wait()
	close(results)

	collected := make([]Result, 0, len(jobs))
	for result := range results {
		collected = append(collected, result)
	}
	sort.Slice(collected, func(i, j int) bool {
		return collected[i].JobID < collected[j].JobID
	})
	return collected, nil
}

// Summarise folds results into Stats.
func Summarise(results []Result) Stats {
	stats := Stats{}
	for _, result := range results {
		if result.Err != nil {
			stats.Failed++
			continue
		}
		stats.Completed++
		if result.Duration > stats.Slowest {
			stats.Slowest = result.Duration
		}
	}
	return stats
}
