<script context="module">
  /** Sort a copy of `rows` by `key`, ascending or descending. */
  export function sortRows(rows, key, ascending = true) {
    const direction = ascending ? 1 : -1;
    return [...rows].sort((a, b) => {
      const left = a[key];
      const right = b[key];
      if (left === right) return 0;
      if (left === null || left === undefined) return 1;
      if (right === null || right === undefined) return -1;
      return left > right ? direction : -direction;
    });
  }

  export const DEFAULT_PAGE_SIZE = 10;
</script>

<script>
  import { createEventDispatcher, onMount, tick } from "svelte";
  import { writable, derived } from "svelte/store";

  export let rows = [];
  export let columns = [];
  export let pageSize = DEFAULT_PAGE_SIZE;
  export let loading = false;
  export let emptyMessage = "Nothing to show";

  const dispatch = createEventDispatcher();

  const query = writable("");
  const page = writable(0);

  let sortKey = columns[0]?.key ?? "";
  let ascending = true;
  let container;

  $: filtered = $query
    ? rows.filter((row) =>
        columns.some((column) =>
          String(row[column.key] ?? "").toLowerCase().includes($query.toLowerCase()),
        ),
      )
    : rows;

  $: sorted = sortKey ? sortRows(filtered, sortKey, ascending) : filtered;
  $: pageCount = Math.max(1, Math.ceil(sorted.length / pageSize));
  $: visible = sorted.slice($page * pageSize, $page * pageSize + pageSize);

  const summary = derived([query, page], ([$query, $page]) =>
    $query ? `filtered, page ${$page + 1}` : `page ${$page + 1}`,
  );

  function toggleSort(key) {
    if (sortKey === key) {
      ascending = !ascending;
    } else {
      sortKey = key;
      ascending = true;
    }
    dispatch("sort", { key: sortKey, ascending });
  }

  async function goTo(next) {
    page.set(Math.min(Math.max(0, next), pageCount - 1));
    await tick();
    container?.scrollTo({ top: 0, behavior: "smooth" });
  }

  function selectRow(row) {
    dispatch("select", row);
  }

  onMount(() => {
    dispatch("ready", { rows: rows.length });
    return () => dispatch("teardown");
  });
</script>

<div class="table" bind:this={container}>
  <header>
    <input
      type="search"
      placeholder="Filter"
      value={$query}
      on:input={(event) => query.set(event.currentTarget.value)}
    />
    <span class="summary">{$summary} of {pageCount}</span>
  </header>

  {#if loading}
    <p class="state">Loading…</p>
  {:else if visible.length === 0}
    <p class="state">{emptyMessage}</p>
  {:else}
    <table>
      <thead>
        <tr>
          {#each columns as column (column.key)}
            <th
              class:sorted={sortKey === column.key}
              on:click={() => toggleSort(column.key)}
            >
              {column.label}
              {#if sortKey === column.key}
                <span aria-hidden="true">{ascending ? "▲" : "▼"}</span>
              {/if}
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        {#each visible as row (row.id)}
          <tr on:click={() => selectRow(row)}>
            {#each columns as column (column.key)}
              <td>{row[column.key] ?? ""}</td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}

  <footer>
    <button disabled={$page === 0} on:click={() => goTo($page - 1)}>Previous</button>
    <button disabled={$page >= pageCount - 1} on:click={() => goTo($page + 1)}>Next</button>
  </footer>
</div>

<style>
  .table {
    border: 1px solid var(--border, #d0d0d0);
    border-radius: 4px;
    font-family: "Segoe UI", system-ui, sans-serif;
    overflow: auto;
  }

  header,
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    background: var(--panel, #f4f4f4);
  }

  table {
    width: 100%;
    border-collapse: collapse;
  }

  th {
    text-align: left;
    cursor: pointer;
    user-select: none;
    padding: 6px 8px;
  }

  th.sorted {
    color: var(--accent, #0061b5);
  }

  td {
    padding: 4px 8px;
    border-top: 1px solid var(--border, #ececec);
  }

  tr:hover td {
    background: var(--hover, #f0f6ff);
  }

  .state {
    padding: 16px;
    text-align: center;
    color: #6b7280;
  }
</style>
