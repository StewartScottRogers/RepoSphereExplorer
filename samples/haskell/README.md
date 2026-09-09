# interpreter

A tiny stack machine: an assembler that says where it gave up, an
interpreter that refuses to run for ever, and a disassembler that
round-trips.

## Using it

```haskell
case assemble "push 2\npush 3\nadd\nprint" of
  Left err      -> putStrLn err
  Right program -> print (runProgram program)
```

## Notes

- `run` stops after `stepLimit` steps and returns a `MachineError`. A
  machine that loops for ever is a machine nobody can embed.
- `assemble` returns `Either String [Instruction]`, and the `String` names
  the line it failed on. "Parse error" with no position is a bug report
  nobody can act on.
- `disassemble . assemble` round-trips, which is the only cheap way to know
  the two agree about the instruction set.

## Building

```bash
cabal build
cabal test
```

---

**This is a fixture.** It lives in `samples/haskell/` so the application has a
Haskell project to open, not just a Haskell file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
