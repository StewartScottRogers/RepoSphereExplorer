# Feed

A paginated feed with a cache in front of it: the loader is a protocol,
the cache is an `actor`, and the caching loader is a decorator over both.

## Using it

```swift
let loader = CachingFeedLoader(
    remote: RemoteFeedLoader(session: .shared),
    cache: InMemoryFeedCache()
)

let page = try await loader.load(after: nil)
for item in page.elements.deduplicated() {
    render(item)
}
```

## Notes

- The cache is an `actor`, so concurrent readers cannot see a half-written
  page. A lock around a dictionary would do the same and would be one more
  thing to get wrong.
- `CachingFeedLoader` conforms to the same `FeedLoading` protocol it wraps,
  so callers cannot tell whether they got a cached page — which is the
  point.
- An empty cache returns an empty array, not an error. Nothing cached yet
  is the ordinary case on first launch.

## Building

```bash
swift build
swift test
```

---

**This is a fixture.** It lives in `samples/swift/` so the application has a
Swift project to open, not just a Swift file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
