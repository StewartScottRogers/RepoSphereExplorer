defmodule RateLimiter do
  @moduledoc """
  A token-bucket rate limiter as a GenServer: each key gets a bucket that
  refills at a steady rate, and a request either takes a token or is told
  how long to wait.
  """

  use GenServer

  require Logger

  @default_capacity 10
  @default_refill_per_second 2.0

  defmodule Bucket do
    @moduledoc "One key's bucket: how many tokens are left, and since when."

    @enforce_keys [:capacity, :tokens, :refill_per_second, :updated_at]
    defstruct [:capacity, :tokens, :refill_per_second, :updated_at]

    @type t :: %__MODULE__{
            capacity: pos_integer(),
            tokens: float(),
            refill_per_second: float(),
            updated_at: integer()
          }
  end

  @type key :: String.t() | atom()
  @type verdict :: :allow | {:deny, milliseconds :: non_neg_integer()}

  # -- client API ----------------------------------------------------------

  @spec start_link(keyword()) :: GenServer.on_start()
  def start_link(opts \\ []) do
    GenServer.start_link(__MODULE__, opts, name: Keyword.get(opts, :name, __MODULE__))
  end

  @doc "Take one token for `key`, or learn how long until one is free."
  @spec check(GenServer.server(), key()) :: verdict()
  def check(server \\ __MODULE__, key) do
    GenServer.call(server, {:check, key})
  end

  @doc "Everything the limiter currently knows, for inspection."
  @spec inspect_buckets(GenServer.server()) :: %{key() => Bucket.t()}
  def inspect_buckets(server \\ __MODULE__) do
    GenServer.call(server, :inspect)
  end

  @doc "Forget one key, releasing its bucket."
  @spec forget(GenServer.server(), key()) :: :ok
  def forget(server \\ __MODULE__, key) do
    GenServer.cast(server, {:forget, key})
  end

  # -- server callbacks ----------------------------------------------------

  @impl GenServer
  def init(opts) do
    state = %{
      buckets: %{},
      capacity: Keyword.get(opts, :capacity, @default_capacity),
      refill: Keyword.get(opts, :refill_per_second, @default_refill_per_second)
    }

    {:ok, state}
  end

  @impl GenServer
  def handle_call({:check, key}, _from, state) do
    bucket = Map.get_lazy(state.buckets, key, fn -> new_bucket(state) end)
    refilled = refill(bucket)

    if refilled.tokens >= 1.0 do
      spent = %{refilled | tokens: refilled.tokens - 1.0}
      {:reply, :allow, put_in(state.buckets[key], spent)}
    else
      wait = wait_millis(refilled)
      Logger.debug("rate limited #{inspect(key)} for #{wait}ms")
      {:reply, {:deny, wait}, put_in(state.buckets[key], refilled)}
    end
  end

  @impl GenServer
  def handle_call(:inspect, _from, state) do
    {:reply, state.buckets, state}
  end

  @impl GenServer
  def handle_cast({:forget, key}, state) do
    {:noreply, %{state | buckets: Map.delete(state.buckets, key)}}
  end

  @impl GenServer
  def handle_info(message, state) do
    Logger.warning("unexpected message: #{inspect(message)}")
    {:noreply, state}
  end

  # -- internals -----------------------------------------------------------

  defp new_bucket(state) do
    %Bucket{
      capacity: state.capacity,
      tokens: state.capacity * 1.0,
      refill_per_second: state.refill,
      updated_at: System.monotonic_time(:millisecond)
    }
  end

  defp refill(%Bucket{} = bucket) do
    now = System.monotonic_time(:millisecond)
    elapsed = (now - bucket.updated_at) / 1000

    tokens =
      (bucket.tokens + elapsed * bucket.refill_per_second)
      |> min(bucket.capacity * 1.0)

    %{bucket | tokens: tokens, updated_at: now}
  end

  defp wait_millis(%Bucket{} = bucket) do
    missing = 1.0 - bucket.tokens
    round(missing / bucket.refill_per_second * 1000)
  end
end

defmodule RateLimiter.Report do
  @moduledoc "Turns a bucket map into something a human can read."

  alias RateLimiter.Bucket

  @spec lines(%{optional(term()) => Bucket.t()}) :: [String.t()]
  def lines(buckets) do
    buckets
    |> Enum.sort_by(fn {key, _} -> to_string(key) end)
    |> Enum.map(fn {key, %Bucket{tokens: tokens, capacity: capacity}} ->
      "#{key}: #{Float.round(tokens, 2)}/#{capacity}"
    end)
  end
end
