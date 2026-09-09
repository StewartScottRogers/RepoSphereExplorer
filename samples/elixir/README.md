# rate_limiter

A token bucket rate limiter as a `GenServer`: each key gets its own
bucket, refilled at a fixed rate, and a refusal tells the caller when to
come back.

## Using it

```elixir
{:ok, _} = RateLimiter.start_link(capacity: 20, refill_per_second: 5.0)

case RateLimiter.check("user:42") do
  :allow -> handle(request)
  {:deny, milliseconds} -> retry_after(milliseconds)
end
```

## Notes

- A refusal is `{:deny, milliseconds}`, never a bare `:deny`. A caller told
  only "no" will retry immediately, which is how a rate limiter becomes a
  busy loop.
- Buckets are per key and created on first sight, so nothing has to be
  registered up front.
- `inspect_buckets/1` exists for tests and for a support engineer looking
  at a live node, and returns a plain map rather than internal state.

## Developing

```bash
mix deps.get
mix test
mix credo --strict
```

---

**This is a fixture.** It lives in `samples/elixir/` so the application has a
Elixir project to open, not just a Elixir file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
