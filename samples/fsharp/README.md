# Parser

A recursive descent parser for arithmetic expressions: a tokeniser, a
parser, and a `Result` at every step rather than an exception.

## Using it

```fsharp
match parse "1 + 2 * 3" with
| Ok expr -> evaluate expr
| Error err -> report err
```

## Notes

- `tokenize` and `parse` both return `Result<_, ParseError>`. A parser that
  throws makes every caller wrap it in a `try`, and most of them will
  forget.
- `ParseError` is a union with a position in it, so an error message can
  point at the character rather than at the whole input.
- `FS0025` — incomplete pattern matches — is a build error. Exhaustiveness
  is most of the reason to write a parser in this language.

## Building

```bash
dotnet build
dotnet test
```

---

**This is a fixture.** It lives in `samples/fsharp/` so the application has a
F# project to open, not just a F# file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
