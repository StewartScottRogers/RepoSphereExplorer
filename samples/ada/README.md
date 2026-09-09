# stack_demo

A bounded stack: generic in its element type, with its own exceptions for
overflow and underflow rather than a status code nobody checks.

## Using it

```ada
package Integer_Stack is new Bounded_Stack (Element => Integer, Capacity => 16);

Integer_Stack.Push (7);
Value := Integer_Stack.Pop;
```

## Notes

- Overflow and underflow raise. A bounded stack that returns a status code
  is a bounded stack whose status code is ignored at three call sites out
  of four.
- Generic in the element type rather than instantiated per type by hand,
  which is what a generic is for.
- Built with `-gnatwe`: every warning is an error. In a language chosen
  for its checking, ignoring the checks is an odd thing to do.

## Building

```bash
alr build
alr run
```

---

**This is a fixture.** It lives in `samples/ada/` so the application has a
Ada project to open, not just a Ada file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
