package pipeline

import (
	"context"
	"errors"
	"strings"
	"testing"
	"time"
)

// upper is the same shape as the package's own handler, written out here so
// the test does not depend on an unexported one.
type upper struct{}

func (upper) Handle(_ context.Context, job Job) (string, error) {
	return strings.ToUpper(job.Payload), nil
}

type refusing struct{}

func (refusing) Handle(_ context.Context, job Job) (string, error) {
	return "", errors.New("refused")
}

func jobs(payloads ...string) []Job {
	built := make([]Job, 0, len(payloads))
	for i, payload := range payloads {
		built = append(built, Job{ID: i + 1, Payload: payload, Timeout: time.Second})
	}
	return built
}

func TestRunReturnsOneResultPerJob(t *testing.T) {
	results, err := Run(context.Background(), 3, jobs("a", "b", "c", "d"), upper{})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if len(results) != 4 {
		t.Fatalf("got %d results, want 4", len(results))
	}
}

func TestRunRefusesAPoolWithNoWorkers(t *testing.T) {
	_, err := Run(context.Background(), 0, jobs("a"), upper{})
	if !errors.Is(err, ErrNoWorkers) {
		t.Fatalf("got %v, want ErrNoWorkers", err)
	}
}

func TestSummariseCountsFailuresSeparately(t *testing.T) {
	results, err := Run(context.Background(), 2, jobs("a", "b"), refusing{})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}

	stats := Summarise(results)
	if stats.Failed != 2 {
		t.Fatalf("Failed = %d, want 2", stats.Failed)
	}
	if stats.Completed != 0 {
		t.Fatalf("Completed = %d, want 0", stats.Completed)
	}
}
