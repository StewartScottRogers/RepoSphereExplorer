"""
Monte-Carlo option pricing with a small type hierarchy, multiple dispatch
and a couple of parallel-friendly kernels - the shapes a Julia preview
should surface: modules, abstract and concrete structs, and methods.
"""
module MonteCarlo

using Printf
using Random
using Statistics

export Option, EuropeanCall, EuropeanPut, AsianCall, MarketState,
       price, payoff, greeks, describe

abstract type Option end

struct MarketState
    spot::Float64
    rate::Float64
    volatility::Float64
    years::Float64
end

struct EuropeanCall <: Option
    strike::Float64
end

struct EuropeanPut <: Option
    strike::Float64
end

struct AsianCall <: Option
    strike::Float64
    observations::Int
end

struct PriceEstimate
    value::Float64
    standard_error::Float64
    paths::Int
end

payoff(option::EuropeanCall, terminal::Float64) = max(terminal - option.strike, 0.0)

payoff(option::EuropeanPut, terminal::Float64) = max(option.strike - terminal, 0.0)

payoff(option::AsianCall, average::Float64) = max(average - option.strike, 0.0)

function terminal_price(market::MarketState, shock::Float64)
    drift = (market.rate - 0.5 * market.volatility^2) * market.years
    diffusion = market.volatility * sqrt(market.years) * shock
    return market.spot * exp(drift + diffusion)
end

function path_average(market::MarketState, rng::AbstractRNG, steps::Int)
    dt = market.years / steps
    spot = market.spot
    total = 0.0
    for _ in 1:steps
        shock = randn(rng)
        drift = (market.rate - 0.5 * market.volatility^2) * dt
        spot *= exp(drift + market.volatility * sqrt(dt) * shock)
        total += spot
    end
    return total / steps
end

function price(option::Option, market::MarketState; paths::Int = 100_000, seed::Int = 20260908)
    rng = MersenneTwister(seed)
    samples = Vector{Float64}(undef, paths)

    for index in 1:paths
        terminal = terminal_price(market, randn(rng))
        samples[index] = payoff(option, terminal)
    end

    discount = exp(-market.rate * market.years)
    value = discount * mean(samples)
    error = discount * std(samples) / sqrt(paths)
    return PriceEstimate(value, error, paths)
end

function price(option::AsianCall, market::MarketState; paths::Int = 20_000, seed::Int = 20260908)
    rng = MersenneTwister(seed)
    samples = [payoff(option, path_average(market, rng, option.observations)) for _ in 1:paths]
    discount = exp(-market.rate * market.years)
    return PriceEstimate(discount * mean(samples), discount * std(samples) / sqrt(paths), paths)
end

function greeks(option::Option, market::MarketState; bump::Float64 = 0.01)
    up = MarketState(market.spot * (1 + bump), market.rate, market.volatility, market.years)
    down = MarketState(market.spot * (1 - bump), market.rate, market.volatility, market.years)

    base = price(option, market).value
    delta = (price(option, up).value - price(option, down).value) / (2 * bump * market.spot)
    gamma = (price(option, up).value - 2 * base + price(option, down).value) /
            (bump * market.spot)^2

    return (delta = delta, gamma = gamma)
end

describe(option::EuropeanCall) = "European call, strike $(option.strike)"
describe(option::EuropeanPut) = "European put, strike $(option.strike)"
describe(option::AsianCall) = "Asian call, strike $(option.strike), $(option.observations) fixings"

function main()
    market = MarketState(100.0, 0.03, 0.22, 1.0)
    book = Option[EuropeanCall(105.0), EuropeanPut(95.0), AsianCall(100.0, 12)]

    for option in book
        estimate = price(option, market)
        @printf("%-42s %8.4f ± %.4f (%d paths)\n",
                describe(option), estimate.value, estimate.standard_error, estimate.paths)
    end

    sensitivities = greeks(book[1], market)
    @printf("call delta %.4f gamma %.6f\n", sensitivities.delta, sensitivities.gamma)
end

end # module

if abspath(PROGRAM_FILE) == @__FILE__
    MonteCarlo.main()
end
