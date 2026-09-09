# csvstats

Reads a comma-separated file and describes its columns: how many values,
what kind they are, and a summary of the numeric ones.

## Using it

```nim
import csvstats

let columns = parse(readFile("readings.csv"))
echo report(columns)
```

Or from the command line:

```bash
nimble build && ./csvstats readings.csv
```

## Notes

- Column kinds are detected from a sample of the rows, not the first one.
  One integer at the top of a column of text is a coincidence, not a type.
- `parse("")` returns no columns rather than raising. An empty file is a
  file, and the caller can say so better than a stack trace can.
- `summarise` is separate from `report`, so a caller who wants the numbers
  does not have to parse them back out of a string.

## Developing

```bash
nimble test
nimble lint
```

---

**This is a fixture.** It lives in `samples/nim/` so the application has a
Nim project to open, not just a Nim file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
