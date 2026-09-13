# authz.rego

Who may read and write the readings, in Open Policy Agent's language.

Rego rules are not statements that run in order: each one either holds
or does not, and the answer is whatever holds. Two things follow, and
both are in this policy:

- A `default` is what the answer is when no rule holds. `default allow
  := false` is the whole reason a policy is safe when it is incomplete.
- A rule written with `contains` is **partial**: every body that holds
  adds to a set rather than deciding it. `deny` here collects every
  reason a write was refused, so the caller is told all of them and not
  just the first.

`within_shift` is a function, which takes an argument; the rest take
their input from `input` and `data`.
