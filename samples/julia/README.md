# MonteCarlo

Monte Carlo option pricing: a market state, a few option payoffs, and an
estimate that carries its own standard error.

## Using it

```julia
using MonteCarlo, Random

market = MarketState(100.0, 0.02, 0.2, 1.0)   # spot, rate, volatility, years
estimate = price(EuropeanCall(100.0), market, MersenneTwister(42); paths=100_000)

println(estimate.value, " ± ", estimate.standard_error)
```

## Notes

- Every function that draws random numbers takes an `AbstractRNG`. A price
  nobody can reproduce is a price nobody can check.
- `PriceEstimate` carries the standard error alongside the value. A Monte
  Carlo number without one is a number with no claim about its accuracy.
- `Option` is an abstract type with concrete payoffs beneath it, so adding
  a payoff means adding a method, not editing a `switch`.

## Developing

```bash
julia --project -e 'using Pkg; Pkg.test()'
```

---

**This is a fixture.** It lives in `samples/julia/` so the application has a
Julia project to open, not just a Julia file. It is valid and
internally consistent, but no pipeline builds it — see `samples/README.md`.
