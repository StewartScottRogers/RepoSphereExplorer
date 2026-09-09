# @example/store

A tiny predictable state container: reducers keyed by action type,
subscribers that only wake when their selection changes, and a subclass
that survives a reload.

## Using it

```js
import { Store } from "@example/store";

const store = new Store({ count: 0 }, {
  increment: (state) => ({ ...state, count: state.count + 1 }),
});

store.subscribe((count) => render(count), (state) => state.count);
store.dispatch({ type: "increment" });
```

## Notes

- An action no reducer handles throws `StoreError`. Silently ignoring it is
  how a typo in an action type survives to production.
- A subscriber takes a selector, and is called only when what it selected
  actually changed. Without that, every subscriber wakes for every action.
- `PersistentStore` takes any storage with `getItem`/`setItem`, so a test
  can hand it a `Map` instead of a browser.

## Developing

```bash
npm test     # node --test, no framework
npm run lint
```

---

**This is a fixture.** It lives in `samples/javascript/` so the application has a
JavaScript project to open, not just a JavaScript file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
