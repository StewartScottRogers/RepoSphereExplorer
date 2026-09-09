defmodule RateLimiterTest do
  use ExUnit.Case, async: true

  doctest RateLimiter

  setup do
    # One name would be shared between asynchronous tests, so each gets its
    # own and passes it explicitly. `name: nil` is not an option: GenServer
    # refuses it, and a test that could not start is worse than none.
    name = :"limiter_#{System.unique_integer([:positive])}"

    limiter =
      start_supervised!({RateLimiter, capacity: 3, refill_per_second: 1.0, name: name})

    %{limiter: limiter}
  end

  test "a fresh key may spend its whole bucket", %{limiter: limiter} do
    for _ <- 1..3 do
      assert :allow = RateLimiter.check(limiter, "alice")
    end
  end

  test "an empty bucket says how long until the next token", %{limiter: limiter} do
    for _ <- 1..3, do: RateLimiter.check(limiter, "bob")

    assert {:deny, milliseconds} = RateLimiter.check(limiter, "bob")
    assert milliseconds > 0, "a refusal that does not say when to retry is a refusal twice"
  end

  test "keys do not spend each other's tokens", %{limiter: limiter} do
    for _ <- 1..3, do: RateLimiter.check(limiter, "carol")

    assert :allow = RateLimiter.check(limiter, "dan")
  end

  test "the buckets can be inspected", %{limiter: limiter} do
    RateLimiter.check(limiter, "erin")

    buckets = RateLimiter.inspect_buckets(limiter)

    assert Map.has_key?(buckets, "erin")
  end
end
