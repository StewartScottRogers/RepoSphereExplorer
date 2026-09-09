/**
 * A minimal observable store: subscribe to slices of state, dispatch plain
 * actions, and get told only when the slice you asked for actually changed.
 */

const IDENTITY = (value) => value;

export class StoreError extends Error {
  constructor(message, action) {
    super(message);
    this.name = "StoreError";
    this.action = action;
  }
}

export class Store {
  #state;
  #reducers;
  #subscribers = new Set();

  constructor(initialState = {}, reducers = {}) {
    this.#state = Object.freeze({ ...initialState });
    this.#reducers = new Map(Object.entries(reducers));
  }

  get state() {
    return this.#state;
  }

  register(type, reducer) {
    if (typeof reducer !== "function") {
      throw new StoreError(`reducer for ${type} is not a function`, type);
    }
    this.#reducers.set(type, reducer);
    return this;
  }

  dispatch(action) {
    if (!action || typeof action.type !== "string") {
      throw new StoreError("an action needs a string type", action);
    }
    const reducer = this.#reducers.get(action.type);
    if (!reducer) {
      throw new StoreError(`no reducer for ${action.type}`, action);
    }
    const previous = this.#state;
    this.#state = Object.freeze(reducer(previous, action));
    this.#notify(previous);
    return this.#state;
  }

  subscribe(listener, selector = IDENTITY) {
    const entry = { listener, selector, last: selector(this.#state) };
    this.#subscribers.add(entry);
    return () => this.#subscribers.delete(entry);
  }

  #notify(previous) {
    for (const entry of this.#subscribers) {
      const next = entry.selector(this.#state);
      if (!Object.is(next, entry.last)) {
        entry.last = next;
        entry.listener(next, entry.selector(previous));
      }
    }
  }
}

export class PersistentStore extends Store {
  constructor(storage, key, initialState, reducers) {
    super(initialState, reducers);
    this.storage = storage;
    this.key = key;
  }

  save() {
    this.storage.setItem(this.key, JSON.stringify(this.state));
  }

  restore(fallback = {}) {
    const raw = this.storage.getItem(this.key);
    return raw ? JSON.parse(raw) : fallback;
  }
}

export function combine(reducers) {
  return (state, action) => {
    let changed = false;
    const next = {};
    for (const [slice, reducer] of Object.entries(reducers)) {
      next[slice] = reducer(state[slice], action);
      changed = changed || next[slice] !== state[slice];
    }
    return changed ? next : state;
  };
}

export function createCounter(initial = 0) {
  return {
    increment: (state = initial, action) => state + (action.by ?? 1),
    reset: () => initial,
  };
}

async function demo() {
  const store = new Store(
    { count: 0, user: null },
    {
      increment: (state, action) => ({ ...state, count: state.count + (action.by ?? 1) }),
      login: (state, action) => ({ ...state, user: action.user }),
    },
  );

  const unsubscribe = store.subscribe(
    (count) => console.log("count is now", count),
    (state) => state.count,
  );

  store.dispatch({ type: "increment" });
  store.dispatch({ type: "increment", by: 4 });
  store.dispatch({ type: "login", user: "ada" });
  unsubscribe();

  return store.state;
}

demo().then((state) => console.log("final", state));
