#!/usr/bin/env python3
"""A tiny in-process task queue with retries, backoff and a dead-letter list.

Written to exercise the shapes a Python preview should surface: dataclasses,
an enum, an abstract base class, decorators, a generator, a context manager
and an async entry point.
"""

from __future__ import annotations

import asyncio
import logging
import random
from abc import ABC, abstractmethod
from contextlib import contextmanager
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from enum import Enum
from typing import Callable, Iterator, Sequence

LOGGER = logging.getLogger(__name__)


class State(Enum):
    """Where a task is in its life."""

    PENDING = "pending"
    RUNNING = "running"
    DONE = "done"
    DEAD = "dead"


@dataclass(slots=True)
class Task:
    """One unit of work, with everything needed to retry it."""

    name: str
    payload: dict[str, object]
    attempts: int = 0
    max_attempts: int = 3
    state: State = State.PENDING
    history: list[str] = field(default_factory=list)

    @property
    def exhausted(self) -> bool:
        return self.attempts >= self.max_attempts

    def record(self, note: str) -> None:
        stamp = datetime.now(timezone.utc).isoformat(timespec="seconds")
        self.history.append(f"{stamp} {note}")


class Backoff(ABC):
    """How long to wait before attempt *n*."""

    @abstractmethod
    def delay(self, attempt: int) -> timedelta:
        """Return the wait before ``attempt``."""


class ExponentialBackoff(Backoff):
    """Doubling delay, with a little jitter so retries do not synchronise."""

    def __init__(self, base: float = 0.05, jitter: float = 0.25) -> None:
        self.base = base
        self.jitter = jitter

    def delay(self, attempt: int) -> timedelta:
        seconds = self.base * (2 ** attempt)
        return timedelta(seconds=seconds * (1 + random.random() * self.jitter))


class ConstantBackoff(Backoff):
    """The same wait every time, which makes tests predictable."""

    def __init__(self, seconds: float = 0.1) -> None:
        self.seconds = seconds

    def delay(self, attempt: int) -> timedelta:
        return timedelta(seconds=self.seconds)


def retrying(handler: Callable[[Task], None]) -> Callable[[Task], bool]:
    """Wrap a handler so a raised exception counts as a failed attempt."""

    def wrapper(task: Task) -> bool:
        try:
            handler(task)
        except Exception as err:  # noqa: BLE001 - the queue decides what is fatal
            task.record(f"failed: {err}")
            return False
        task.record("succeeded")
        return True

    return wrapper


@contextmanager
def draining(queue: "TaskQueue") -> Iterator["TaskQueue"]:
    """Run a block, then report what the queue could not finish."""

    try:
        yield queue
    finally:
        for task in queue.dead_letters:
            LOGGER.warning("dead letter: %s after %d attempts", task.name, task.attempts)


class TaskQueue:
    """A queue that retries failing tasks until they are exhausted."""

    def __init__(self, backoff: Backoff | None = None) -> None:
        self._tasks: list[Task] = []
        self._backoff = backoff or ExponentialBackoff()
        self.dead_letters: list[Task] = []

    def __len__(self) -> int:
        return len(self._tasks)

    def submit(self, name: str, **payload: object) -> Task:
        task = Task(name=name, payload=payload)
        self._tasks.append(task)
        return task

    def pending(self) -> Iterator[Task]:
        for task in self._tasks:
            if task.state is State.PENDING:
                yield task

    async def run(self, handler: Callable[[Task], None]) -> Sequence[Task]:
        wrapped = retrying(handler)
        while any(task.state is State.PENDING for task in self._tasks):
            for task in list(self.pending()):
                task.state = State.RUNNING
                task.attempts += 1
                if wrapped(task):
                    task.state = State.DONE
                elif task.exhausted:
                    task.state = State.DEAD
                    self.dead_letters.append(task)
                else:
                    task.state = State.PENDING
                    await asyncio.sleep(self._backoff.delay(task.attempts).total_seconds())
        return tuple(self._tasks)


def flaky(task: Task) -> None:
    """A handler that refuses the first two attempts of any task named 'flaky'."""

    if task.name == "flaky" and task.attempts < 3:
        raise RuntimeError(f"attempt {task.attempts} refused")


async def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    queue = TaskQueue(backoff=ConstantBackoff(0.01))
    queue.submit("steady", value=1)
    queue.submit("flaky", value=2)
    queue.submit("doomed", value=3)

    with draining(queue) as running:
        finished = await running.run(flaky)

    for task in finished:
        print(f"{task.name:<8} {task.state.value:<8} attempts={task.attempts}")


if __name__ == "__main__":
    asyncio.run(main())
