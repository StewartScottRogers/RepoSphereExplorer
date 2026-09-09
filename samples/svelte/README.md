# @example/data-table

A sortable, filterable data table: give it rows, get a table.

## Using it

```svelte
<script>
  import { DataTable } from "@example/data-table";

  let rows = $state([
    { id: 1, name: "Ada", score: 91 },
    { id: 2, name: "Grace", score: 88 },
  ]);
</script>

<DataTable {rows} />
```

## Notes

- Runes are on, so reactivity is declared rather than inferred from where
  an assignment happens to be. In a component that sorts and filters, the
  difference is the difference between a table and a puzzle.
- The package exports through `src/lib/index.js`, so the file layout stays
  the library's to change without breaking every import.
- An empty `rows` array renders an empty table, not a crash. That is the
  first state every table is in and the one least likely to be tested.

## Developing

```bash
npm test
npm run check
```

---

**This is a fixture.** It lives in `samples/svelte/` so the application has a
Svelte project to open, not just a Svelte file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
