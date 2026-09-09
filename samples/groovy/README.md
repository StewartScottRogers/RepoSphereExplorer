# build-report

Turns a pile of build results into a report somebody can read: grouped by
project, failures pulled out, and a total that includes the runs that
failed.

## Using it

```groovy
def report = new ReportBuilder()
results.each { report.add(it) }

new ReportPrinter().print(report)
println "${report.failures().size()} failure(s) in ${report.total()}"
```

## Notes

- `total()` counts failed runs too. A build report that hides the time
  spent failing is a build report that says the pipeline is fast.
- `ReportBuilder` is `@CompileStatic`, so the parts that run per result are
  compiled rather than dispatched dynamically.
- Printing is a separate class from building, so a test can assert on the
  numbers without capturing standard output.

## Building

```bash
./gradlew test
```

---

**This is a fixture.** It lives in `samples/groovy/` so the application has a
Groovy project to open, not just a Groovy file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
