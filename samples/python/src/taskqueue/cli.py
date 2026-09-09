"""The ``taskqueue`` command: submit some tasks and run them once.

Argument parsing only. Everything it calls lives in the package, so it can
be tested without a process.
"""

from __future__ import annotations

import argparse
import asyncio
import sys

from taskqueue.queue import ConstantBackoff, ExponentialBackoff, State, TaskQueue, flaky


def _queue(args: argparse.Namespace) -> TaskQueue:
    if args.backoff == "constant":
        return TaskQueue(ConstantBackoff(args.delay_ms / 1000))
    return TaskQueue(ExponentialBackoff())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="taskqueue")
    parser.add_argument("names", nargs="+", help="task names to submit")
    parser.add_argument(
        "--backoff",
        choices=("exponential", "constant"),
        default="exponential",
        help="how long to wait between attempts",
    )
    parser.add_argument(
        "--delay-ms", type=int, default=10, help="constant backoff delay, in milliseconds"
    )
    parser.add_argument(
        "--max-attempts", type=int, default=3, help="how many times to try each task"
    )
    args = parser.parse_args(argv)

    queue = _queue(args)
    for name in args.names:
        task = queue.submit(name)
        task.max_attempts = args.max_attempts

    finished = asyncio.run(queue.run(flaky))

    for task in finished:
        print(f"{task.state.name.lower():<8} {task.name} after {task.attempts} attempt(s)")
    if queue.dead_letters:
        print(f"{len(queue.dead_letters)} dead letter(s)", file=sys.stderr)
        return 1
    return 0 if all(task.state is State.DONE for task in finished) else 1


if __name__ == "__main__":
    sys.exit(main())
