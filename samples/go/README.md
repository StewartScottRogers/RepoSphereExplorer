# pipeline

A bounded pool of workers over a slice of jobs, cancelling everything if
any one of them fails, and a summary of what happened.

## Using it

```go
results, err := pipeline.Run(ctx, runtime.NumCPU(), jobs, handler)
if err != nil {
    return err
}
stats := pipeline.Summarise(results)
```

Or from the command line:

```bash
make build && ./build/pipeline -workers 8 alpha beta gamma
```

## Notes

- `Run` returns `ErrNoWorkers` rather than clamping. A pool asked for zero
  workers is a caller bug, and quietly giving them one hides it.
- Results come back in job order, not completion order: a caller who wanted
  completion order would want a channel instead.
- Every handler must respect the context it is given. `Run` cancels the
  rest as soon as one fails, and a handler that ignores cancellation makes
  that promise a lie.

---

**This is a fixture.** It lives in `samples/go/` so the application has a
Go project to open, not just a Go file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
