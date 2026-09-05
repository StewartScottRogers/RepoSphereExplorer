import std/strformat

proc greet(name: string): string =
  &"Hello, {name}"

echo greet("World")
