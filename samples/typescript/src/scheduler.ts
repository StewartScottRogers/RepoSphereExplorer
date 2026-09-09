/**
 * A cooperative scheduler: tasks declare what they depend on, the scheduler
 * runs them in dependency order with a bounded amount of concurrency, and
 * reports a cycle rather than deadlocking on one.
 */

export type TaskId = string;

export type TaskState = "waiting" | "running" | "done" | "failed";

export interface TaskSpec<T> {
  readonly id: TaskId;
  readonly dependsOn?: readonly TaskId[];
  run(context: RunContext): Promise<T>;
}

export interface RunContext {
  readonly attempt: number;
  readonly results: ReadonlyMap<TaskId, unknown>;
  signal: AbortSignal;
}

export interface SchedulerOptions {
  concurrency?: number;
  retries?: number;
  onStateChange?: (id: TaskId, state: TaskState) => void;
}

export interface RunSummary {
  readonly completed: readonly TaskId[];
  readonly failed: ReadonlyMap<TaskId, Error>;
  readonly durationMs: number;
}

export class CycleError extends Error {
  constructor(public readonly cycle: readonly TaskId[]) {
    super(`dependency cycle: ${cycle.join(" -> ")}`);
    this.name = "CycleError";
  }
}

export class UnknownDependencyError extends Error {
  constructor(
    public readonly task: TaskId,
    public readonly missing: TaskId,
  ) {
    super(`task ${task} depends on unknown task ${missing}`);
    this.name = "UnknownDependencyError";
  }
}

type Node<T> = {
  spec: TaskSpec<T>;
  state: TaskState;
  dependents: Set<TaskId>;
  remaining: number;
};

export class Scheduler<T = unknown> {
  private readonly nodes = new Map<TaskId, Node<T>>();
  private readonly results = new Map<TaskId, unknown>();
  private readonly concurrency: number;
  private readonly retries: number;
  private readonly onStateChange: NonNullable<SchedulerOptions["onStateChange"]>;

  constructor(options: SchedulerOptions = {}) {
    this.concurrency = Math.max(1, options.concurrency ?? 4);
    this.retries = Math.max(0, options.retries ?? 0);
    this.onStateChange = options.onStateChange ?? (() => undefined);
  }

  add(spec: TaskSpec<T>): this {
    this.nodes.set(spec.id, {
      spec,
      state: "waiting",
      dependents: new Set(),
      remaining: spec.dependsOn?.length ?? 0,
    });
    return this;
  }

  private link(): void {
    for (const [id, node] of this.nodes) {
      for (const dependency of node.spec.dependsOn ?? []) {
        const upstream = this.nodes.get(dependency);
        if (!upstream) {
          throw new UnknownDependencyError(id, dependency);
        }
        upstream.dependents.add(id);
      }
    }
  }

  private ready(): TaskId[] {
    return [...this.nodes.entries()]
      .filter(([, node]) => node.state === "waiting" && node.remaining === 0)
      .map(([id]) => id);
  }

  private transition(id: TaskId, state: TaskState): void {
    const node = this.nodes.get(id);
    if (node) {
      node.state = state;
      this.onStateChange(id, state);
    }
  }

  async run(signal: AbortSignal = new AbortController().signal): Promise<RunSummary> {
    const started = Date.now();
    const failed = new Map<TaskId, Error>();
    const completed: TaskId[] = [];
    this.link();

    let running: Promise<void>[] = [];
    let ready = this.ready();

    while (ready.length > 0 || running.length > 0) {
      while (ready.length > 0 && running.length < this.concurrency) {
        const id = ready.shift() as TaskId;
        running.push(this.execute(id, signal, completed, failed));
      }
      await Promise.race(running);
      running = running.filter((promise) => this.isPending(promise));
      ready = this.ready();
    }

    const stuck = [...this.nodes.values()].filter((node) => node.state === "waiting");
    if (stuck.length > 0 && failed.size === 0) {
      throw new CycleError(stuck.map((node) => node.spec.id));
    }

    return { completed, failed, durationMs: Date.now() - started };
  }

  private isPending(promise: Promise<void>): boolean {
    return this.pending.has(promise);
  }

  private readonly pending = new Set<Promise<void>>();

  private async execute(
    id: TaskId,
    signal: AbortSignal,
    completed: TaskId[],
    failed: Map<TaskId, Error>,
  ): Promise<void> {
    const node = this.nodes.get(id);
    if (!node) {
      return;
    }
    this.transition(id, "running");

    for (let attempt = 0; attempt <= this.retries; attempt += 1) {
      try {
        const value = await node.spec.run({ attempt, results: this.results, signal });
        this.results.set(id, value);
        completed.push(id);
        this.transition(id, "done");
        for (const dependent of node.dependents) {
          const downstream = this.nodes.get(dependent);
          if (downstream) {
            downstream.remaining -= 1;
          }
        }
        return;
      } catch (error) {
        if (attempt === this.retries) {
          failed.set(id, error as Error);
          this.transition(id, "failed");
        }
      }
    }
  }
}

export function describe(summary: RunSummary): string {
  const failures = [...summary.failed.keys()];
  return failures.length === 0
    ? `${summary.completed.length} tasks in ${summary.durationMs}ms`
    : `${failures.length} failed: ${failures.join(", ")}`;
}
