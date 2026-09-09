"""An in-process task queue with retries, backoff and dead letters.

The queue itself is in :mod:`taskqueue.queue`. This is the front door: the
names most callers want, and nothing else.
"""

from taskqueue.queue import (
    Backoff,
    ConstantBackoff,
    ExponentialBackoff,
    State,
    Task,
    TaskQueue,
    draining,
    retrying,
)

__all__ = [
    "Backoff",
    "ConstantBackoff",
    "ExponentialBackoff",
    "State",
    "Task",
    "TaskQueue",
    "__version__",
    "draining",
    "retrying",
]

__version__ = "0.9.2"
