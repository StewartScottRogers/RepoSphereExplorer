# DownloadQueue

A bounded download queue: a fixed number run at once, failures are
retried, and progress is reported as it goes.

## Using it

```objc
DownloadQueue *queue = [[DownloadQueue alloc] initWithMaxConcurrent:4];

for (NSURL *url in urls) {
    [queue enqueue:[[DownloadTask alloc] initWithURL:url maxAttempts:3]];
}

[queue waitUntilFinished];
```

## Notes

- `init` is `NS_UNAVAILABLE` on both classes. A queue with no concurrency
  limit and a task with no URL are not useful objects, and letting them be
  constructed just moves the failure later.
- `NS_ASSUME_NONNULL_BEGIN` covers the header, so the few things that may
  be nil say so explicitly.
- Retries are per task, not per queue: one bad URL should not consume the
  whole queue's patience.

## Building

```bash
pod lib lint
```

---

**This is a fixture.** It lives in `samples/objective-c/` so the application has a
Objective-C project to open, not just a Objective-C file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
