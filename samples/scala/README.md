# event-stream

An append-only event store and the projections that read it: events are a
sealed trait, failures are values, and folding a stream to a state is one
function.

## Using it

```scala
val store = InMemoryEventStore()
store.append(AccountOpened("acc-1", owner = "Ada", at = now))
store.append(MoneyDeposited("acc-1", pennies = 2500L, at = now))

val state = AccountProjection.fold("acc-1", store.read(Offset(0)).getOrElse(Nil).map(_.event))
```

## Notes

- Money is `Long` pennies inside and `BigDecimal` only at the edge. A
  balance that drifts by a rounding error is a balance nobody can audit.
- `read` returns `Either[StreamError, ...]`. Clamping a too-old offset to
  the earliest one would silently hand a reader a different answer from the
  one it asked for.
- `Event` is sealed, so a new event type that a projection forgets to
  handle is a compile error rather than a runtime surprise.

## Building

```bash
sbt test
sbt scalafmtCheckAll
```

---

**This is a fixture.** It lives in `samples/scala/` so the application has a
Scala project to open, not just a Scala file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
