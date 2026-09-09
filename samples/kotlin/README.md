# library-loans

A small lending library: value classes for identifiers, a sealed result
for lending, and loans that arrive as a `Flow`.

## Using it

```kotlin
val library = InMemoryLibrary(catalogue)

when (val result = library.lend(Isbn("9780441013593"), memberId = 42)) {
    is LoanResult.Granted -> confirm(result.loan)
    is LoanResult.Refused -> explain(result.reason)
    LoanResult.AlreadyHeld -> remind(member)
}

library.loans(memberId = 42).collect { render(it) }
```

## Notes

- `Isbn` is a `@JvmInline value class`, so a member identifier and a book
  identifier cannot be swapped by accident and nothing is boxed to get
  that.
- `availability` distinguishes `ALL_ON_LOAN` from `UNKNOWN_BOOK`. "We do
  not have that book" and "somebody else has it" need different answers.
- Lending returns a sealed `LoanResult` rather than throwing. A refusal is
  an ordinary outcome, not a failure.

## Building

```bash
./gradlew test
./gradlew koverHtmlReport
```

---

**This is a fixture.** It lives in `samples/kotlin/` so the application has a
Kotlin project to open, not just a Kotlin file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
