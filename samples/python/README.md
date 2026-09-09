# taskqueue

An in-process task queue with retries, backoff and a dead-letter list. No
broker and no daemon: the queue is an object, and running it is one
`await`.

## Using it

```python
import asyncio
from taskqueue import TaskQueue

queue = TaskQueue()
queue.submit("resize", path="image-1.png")
queue.submit("thumbnail", path="image-1.png", width=128)

finished = asyncio.run(queue.run(handler))
print(len(queue.dead_letters), "gave up")
```

Or from the command line:

```bash
taskqueue resize thumbnail --backoff constant --delay-ms 50
```

## Notes

- A task that fails is retried until `max_attempts`, then moved to
  `dead_letters` rather than dropped. A queue that silently loses work is
  worse than one that stops.
- Backoff is a strategy object, so a test can pass `ConstantBackoff` and
  not spend its life asleep waiting for an exponential one.
- `retrying` is a decorator and `draining` a context manager, both over the
  same queue: the module is a tour of the shapes Python code takes.

---

**This is a fixture.** It lives in `samples/python/` so the application has a
Python project to open, not just a Python file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
