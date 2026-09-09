# http_cache

A bounded HTTP response cache: entries know when they go stale, the
capacity is enforced rather than suggested, and a cache that could never
hold anything is refused at construction.

## Using it

```crystal
store = HttpCache::Store.new(capacity: 256)

store.put("/orders/1", body, max_age: 60.seconds)

if cached = store.get("/orders/1")
  respond(cached)
else
  respond(fetch("/orders/1"))
end
```

## Notes

- `capacity: 0` raises `CapacityError` at construction. A cache that can
  never hold anything is a bug at its call site, not a cache.
- Freshness is per entry, from the response, not a single time-to-live for
  the whole store. Two endpoints rarely go stale at the same rate.
- `CapacityError` is a subclass of `CacheError`, so a caller can rescue the
  family or the specific one.

## Developing

```bash
shards install
crystal spec
bin/ameba
```

---

**This is a fixture.** It lives in `samples/crystal/` so the application has a
Crystal project to open, not just a Crystal file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
