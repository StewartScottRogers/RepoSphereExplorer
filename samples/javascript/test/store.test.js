// Run with `node --test test/`, which needs no test framework at all.

import assert from "node:assert/strict";
import test from "node:test";

import { PersistentStore, Store, StoreError, combine, createCounter } from "../src/store.js";

test("a dispatched action reaches its reducer", () => {
  const store = new Store({ count: 0 }, { increment: (state) => ({ count: state.count + 1 }) });

  store.dispatch({ type: "increment" });

  assert.equal(store.state.count, 1);
});

test("an action with no reducer is refused rather than ignored", () => {
  const store = new Store({ count: 0 });

  assert.throws(() => store.dispatch({ type: "nothing-handles-this" }), StoreError);
});

test("a subscriber is told only when its selection changes", () => {
  const store = new Store({ count: 0, unrelated: "x" });
  store.register("increment", (state) => ({ ...state, count: state.count + 1 }));
  store.register("touch", (state) => ({ ...state, unrelated: "y" }));

  const seen = [];
  store.subscribe((count) => seen.push(count), (state) => state.count);

  store.dispatch({ type: "touch" });
  store.dispatch({ type: "increment" });

  assert.deepEqual(seen, [1], "the unrelated change must not wake the subscriber");
});

test("combine runs every reducer it was given", () => {
  const reducer = combine({
    count: (state) => state + 1,
    label: (state) => `${state}!`,
  });

  assert.deepEqual(reducer({ count: 1, label: "a" }, { type: "any" }), {
    count: 2,
    label: "a!",
  });
});

test("a persistent store comes back from its storage", () => {
  const storage = new Map();
  const saved = new PersistentStore(storage, "counter", { count: 7 });
  saved.save();

  const restored = new PersistentStore(storage, "counter", { count: 0 });

  assert.equal(restored.restore().count, 7);
});

test("the counter helper starts where it was told to", () => {
  const counter = createCounter(10);

  assert.equal(counter.state.count, 10);
});
