import { test } from "node:test";
import assert from "node:assert/strict";
import { buildCli, ReportError } from "../src/index.js";

test("the CLI names itself after the floor", () => {
  const program = buildCli();
  assert.equal(program.name(), "factory-floor");
});

test("a report error carries its exit code", () => {
  const error = new ReportError("no repositories found", 2);
  assert.equal(error.exitCode, 2);
  assert.equal(error.name, "ReportError");
});
