defmodule Hello do
  @moduledoc """
  A tiny greeting module.
  """

  def greet(name) do
    "Hello, #{name}!"
    |> IO.puts()
  end
end

Hello.greet("world")
