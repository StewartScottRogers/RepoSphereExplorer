"""The queue at its front door, as a caller meets it."""

from __future__ import annotations

import pytest

from taskqueue import ConstantBackoff, State, TaskQueue


@pytest.fixture()
def queue() -> TaskQueue:
    # Constant and tiny: an exponential backoff would make this test spend
    # most of its life asleep.
    return TaskQueue(ConstantBackoff(0.001))


def test_a_submitted_task_starts_pending(queue: TaskQueue) -> None:
    task = queue.submit("resize", path="image-1.png")

    assert task.state is State.PENDING
    assert task.payload == {"path": "image-1.png"}
    assert list(queue.pending()) == [task]
    assert len(queue) == 1


async def test_every_task_finishes_when_the_handler_succeeds(queue: TaskQueue) -> None:
    queue.submit("one")
    queue.submit("two")

    finished = await queue.run(lambda task: None)

    assert [task.state for task in finished] == [State.DONE, State.DONE]
    assert queue.dead_letters == []


async def test_a_task_that_never_succeeds_becomes_a_dead_letter(queue: TaskQueue) -> None:
    def always_fails(task: object) -> None:
        raise RuntimeError("no")

    task = queue.submit("doomed")
    task.max_attempts = 2

    await queue.run(always_fails)

    assert task.state is State.DEAD
    assert queue.dead_letters == [task]
    assert task.attempts == 2


async def test_a_task_is_retried_until_it_succeeds(queue: TaskQueue) -> None:
    attempts: list[int] = []

    def succeeds_on_the_third_try(task: object) -> None:
        attempts.append(1)
        if len(attempts) < 3:
            raise RuntimeError("not yet")

    task = queue.submit("flaky")
    task.max_attempts = 5

    await queue.run(succeeds_on_the_third_try)

    assert task.state is State.DONE
    assert task.attempts == 3
    assert task.history, "each attempt should leave a note behind"
