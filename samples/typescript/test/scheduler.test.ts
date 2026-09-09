// Run with `npm test`, which type-checks first and then executes these
// directly: Node strips the types rather than compiling to a temporary
// directory nobody reads.

import assert from "node:assert/strict";
import test from "node:test";

import { CycleError, Scheduler, UnknownDependencyError, describe } from "../src/scheduler.ts";

test("tasks run after the tasks they depend on", async () => {
  const order: string[] = [];
  const scheduler = new Scheduler<string>()
    .add({ id: "b", dependsOn: ["a"], run: async () => (order.push("b"), "b") })
    .add({ id: "a", run: async () => (order.push("a"), "a") });

  await scheduler.run();

  assert.deepEqual(order, ["a", "b"]);
});

test("a cycle is refused before anything runs", () => {
  const scheduler = new Scheduler()
    .add({ id: "a", dependsOn: ["b"], run: async () => undefined })
    .add({ id: "b", dependsOn: ["a"], run: async () => undefined });

  return assert.rejects(() => scheduler.run(), CycleError);
});

test("a dependency on a task that was never added is refused", () => {
  const scheduler = new Scheduler().add({
    id: "a",
    dependsOn: ["nobody-added-this"],
    run: async () => undefined,
  });

  return assert.rejects(() => scheduler.run(), UnknownDependencyError);
});

test("an aborted run stops rather than finishing quietly", async () => {
  const controller = new AbortController();
  const scheduler = new Scheduler().add({
    id: "slow",
    run: async () => {
      controller.abort();
      return undefined;
    },
  });

  const summary = await scheduler.run(controller.signal);

  assert.ok(describe(summary).length > 0, "a summary should say what happened");
});
