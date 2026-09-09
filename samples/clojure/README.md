# ledger

Double-entry bookkeeping in the small: postings, entries that must
balance, and a trial balance that sums to zero or tells you it does not.

## Using it

```clojure
(require '[ledger.core :as ledger])

(def sale
  (ledger/entry "2026-01-01" "Sale"
                [(ledger/posting "assets:cash" 100)
                 (ledger/posting "income:sales" -100)]))

(ledger/balance [sale] "assets:cash")   ;=> 100
(ledger/trial-balance [sale])           ;=> {"assets:cash" 100, "income:sales" -100}
```

## Notes

- `entry` refuses postings that do not sum to zero. An unbalanced entry is
  not a rounding problem to fix later; it is a mistake to catch now.
- Account types come from the first colon-separated segment, so
  `assets:cash:current` is an asset without anybody maintaining a list.
- A trial balance that does not sum to zero means the books are wrong, and
  the function returns the numbers rather than asserting, so a caller can
  show them.

## Developing

```bash
clojure -X:test
clojure -M:lint
```

---

**This is a fixture.** It lives in `samples/clojure/` so the application has a
Clojure project to open, not just a Clojure file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
