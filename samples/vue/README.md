# @example/task-board

A task board: columns of cards, reordered by dragging, with the state kept
in the component rather than in a store nobody asked for.

## Using it

```bash
npm install
npm run dev
```

## Notes

- `src/main.js` mounts the component and does nothing else. Logic in a
  mount script is logic no test can reach.
- No state management library. One board with one list of cards does not
  need a store, and adding one before it does makes every later decision
  harder to unpick.
- `vue/require-default-prop` is an error, so an optional prop has to say
  what it is when nobody passes it.

## Developing

```bash
npm test
npm run lint
npm run build
```

---

**This is a fixture.** It lives in `samples/vue/` so the application has a
Vue project to open, not just a Vue file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
