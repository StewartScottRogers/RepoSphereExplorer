# lru

A bounded least-recently-used cache, built as a functor so the key type
and its comparison come from the caller.

## Using it

```ocaml
module Cache = Lru.Make (struct
  type t = string
  let compare = String.compare
  let to_string k = k
end)

let cache = Cache.create ~capacity:128 in
Cache.put cache "key" value;
match Cache.get cache "key" with
| Some v -> use v
| None -> miss ()
```

## Notes

- A functor rather than a polymorphic cache, so the comparison is chosen
  once at instantiation rather than passed at every call.
- `capacity` is required, not defaulted. An unbounded LRU cache is a
  memory leak with extra steps.
- `hit_rate` is part of the interface, because a cache whose hit rate you
  cannot see is a cache you cannot tune.

## Building

```bash
dune build
dune runtest
```

---

**This is a fixture.** It lives in `samples/ocaml/` so the application has a
OCaml project to open, not just a OCaml file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
