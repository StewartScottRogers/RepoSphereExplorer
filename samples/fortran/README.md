# heat

An explicit finite-difference solver for the one-dimensional heat
equation.

## Using it

```fortran
use heat_kernel

real(kind=8) :: field(128)
field = initial_condition()

do n = 1, steps
    call step(field, alpha)
end do
```

## Notes

- Explicit time stepping, so the diffusion number has to stay below one
  half or the solution oscillates and then explodes. That is a property of
  the scheme, not a bug, and the module documents it rather than hiding it
  behind an automatic step size.
- `implicit none` everywhere, and `implicit-typing = false` in `fpm.toml`
  so a new file cannot forget it. Implicit typing is how a typo becomes a
  silent zero.
- Boundaries are held fixed. A solver that quietly wraps around is solving
  a different problem from the one you posed.

## Building

```bash
fpm build
fpm test
```

---

**This is a fixture.** It lives in `samples/fortran/` so the application has a
Fortran project to open, not just a Fortran file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
