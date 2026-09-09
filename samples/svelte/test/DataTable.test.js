import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import DataTable from "../src/lib/DataTable.svelte";

afterEach(cleanup);

const rows = [
  { id: 1, name: "Ada", score: 91 },
  { id: 2, name: "Grace", score: 88 },
  { id: 3, name: "Alan", score: 79 },
];

describe("DataTable", () => {
  it("renders a row for each item it was given", () => {
    const { container } = render(DataTable, { props: { rows } });

    expect(container.querySelectorAll("tbody tr").length).toBe(rows.length);
  });

  it("renders a header rather than a bare grid of values", () => {
    const { container } = render(DataTable, { props: { rows } });

    expect(container.querySelector("thead")).not.toBeNull();
  });

  it("renders without falling over when there is nothing to show", () => {
    // An empty table is the ordinary first state of every table, and it is
    // the one most likely to be left untested.
    const { container } = render(DataTable, { props: { rows: [] } });

    expect(container.querySelectorAll("tbody tr").length).toBe(0);
  });
});
