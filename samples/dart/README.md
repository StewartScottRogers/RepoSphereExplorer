# weather

A weather client: readings that describe themselves, a source you can
fake, and a decorator that retries a flaky one.

## Using it

```dart
final source = RetryingSource(HttpSource(apiKey), attempts: 3);
final reading = await source.current('LHR');

print(reading.summary); // LHR 12.3C clear
```

## Notes

- `RetryingSource` wraps any `WeatherSource` rather than being built into
  one. Retrying is a policy, and a policy baked into a client is a policy
  nobody can turn off.
- It gives up after `attempts` and throws `WeatherException`. Retrying for
  ever turns one broken endpoint into a broken process.
- `FakeSource` ships in the library, not the tests, because everybody who
  depends on this needs one too.

## Developing

```bash
dart pub get
dart test
dart analyze
```

---

**This is a fixture.** It lives in `samples/dart/` so the application has a
Dart project to open, not just a Dart file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
