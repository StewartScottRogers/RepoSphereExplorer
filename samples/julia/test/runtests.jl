using MonteCarlo
using Test

@testset "MonteCarlo" begin
    # Positional, as the struct declares it: spot, rate, volatility, years.
    market = MarketState(100.0, 0.02, 0.2, 1.0)

    @testset "payoff" begin
        @test payoff(EuropeanCall(100.0), 120.0) == 20.0
        @test payoff(EuropeanCall(100.0), 80.0) == 0.0
        @test payoff(EuropeanPut(100.0), 80.0) == 20.0
        @test payoff(EuropeanPut(100.0), 120.0) == 0.0
    end

    @testset "price" begin
        estimate = price(EuropeanCall(100.0), market; paths = 4_000)

        @test estimate.value > 0
        @test estimate.standard_error > 0
        @test estimate.paths == 4_000
    end

    @testset "the same seed gives the same answer" begin
        # A price nobody can reproduce is a price nobody can check.
        a = price(EuropeanCall(100.0), market; paths = 2_000, seed = 7)
        b = price(EuropeanCall(100.0), market; paths = 2_000, seed = 7)

        @test a.value == b.value
    end

    @testset "a call is worth more the further in the money it is" begin
        deep = price(EuropeanCall(80.0), market; paths = 4_000, seed = 11)
        shallow = price(EuropeanCall(120.0), market; paths = 4_000, seed = 11)

        @test deep.value > shallow.value
    end

    @testset "an Asian option averages over its observations" begin
        estimate = price(AsianCall(100.0, 12), market; paths = 1_000)

        @test isfinite(estimate.value)
    end

    @testset "greeks come back named, so nobody reads them by position" begin
        sensitivities = greeks(EuropeanCall(100.0), market)

        @test haskey(sensitivities, :delta)
        @test haskey(sensitivities, :gamma)
    end

    @testset "describe names the option a reader is looking at" begin
        @test occursin("call", describe(EuropeanCall(100.0)))
        @test occursin("put", describe(EuropeanPut(100.0)))
        @test occursin("fixings", describe(AsianCall(100.0, 12)))
    end
end
