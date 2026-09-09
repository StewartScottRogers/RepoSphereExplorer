# supervisor_demo

A small worker pool behind a `gen_server`: submit work, ask for
statistics, drain it before you stop it.

## Using it

```erlang
{ok, Pool} = supervisor_demo:start_link(4),
ok = supervisor_demo:submit(Pool, fun() -> expensive() end),
Stats = supervisor_demo:stats(Pool),
_ = supervisor_demo:drain(Pool),
ok = supervisor_demo:stop(Pool).
```

## Notes

- `drain/1` exists so a caller can wait for outstanding work before
  stopping. A pool that drops queued work on shutdown loses it silently.
- `stats/1` returns a term, not a printed line, so a supervisor tree can
  act on it rather than a human reading logs.
- `warnings_as_errors` and `warn_missing_spec` are on: an exported function
  without a spec does not compile here.

## Building

```bash
rebar3 compile
rebar3 ct
rebar3 dialyzer
```

---

**This is a fixture.** It lives in `samples/erlang/` so the application has a
Erlang project to open, not just a Erlang file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
