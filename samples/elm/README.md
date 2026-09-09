# budget

A household budget in the Elm architecture: a model, a message type that
covers every way the model can change, and a view that renders it.

## Using it

```bash
elm make src/Main.elm --output dist/main.js
elm reactor            # then open src/Main.elm
```

## Notes

- Money is pennies as `Int`. Elm has no decimal type, and a budget in
  `Float` pounds will disagree with itself over a long enough month.
- `Msg` is a custom type, so a new message the update function forgets to
  handle is a compile error. That is most of why this is written in Elm.
- `Main` is `Browser.sandbox` and nothing else. Every decision lives in
  `Budget`, where a test can reach it without a browser.

## Developing

```bash
elm-test
elm-format --validate src tests
```

---

**This is a fixture.** It lives in `samples/elm/` so the application has a
Elm project to open, not just a Elm file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
