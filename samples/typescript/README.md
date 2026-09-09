# @example/scheduler

A dependency-ordered task scheduler: declare what each task needs, and it
works out the order, runs what it can in parallel, and stops on an
`AbortSignal`.

## Using it

```ts
const scheduler = new Scheduler<Buffer>()
  .add({ id: "fetch", run: fetchSource })
  .add({ id: "compile", dependsOn: ["fetch"], run: compile })
  .add({ id: "bundle", dependsOn: ["compile"], run: bundle });

const summary = await scheduler.run(controller.signal);
console.log(describe(summary));
```

## Notes

- A cycle throws `CycleError` **before** anything runs. Discovering a cycle
  halfway through leaves you with half a build and no way to reason about
  it.
- A dependency on a task that was never added throws
  `UnknownDependencyError` rather than being treated as already satisfied.
- `run` takes an `AbortSignal`, not a boolean flag, so cancellation
  composes with everything else that takes one.

## Developing

```bash
npm run build
npm test        # type-checks, then runs
```

---

**This is a fixture.** It lives in `samples/typescript/` so the application has a
TypeScript project to open, not just a TypeScript file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
