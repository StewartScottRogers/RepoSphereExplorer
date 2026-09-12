//// Summary statistics for a column of readings.
////
//// `describe` is public and its signature says nothing about what comes
//// back, so a caller has to read the body to find out. It is left that
//// way so the file pane has something to warn about.

import gleam/float
import gleam/int
import gleam/list
import gleam/result
import gleam/string

/// A named column of readings.
pub type Column {
  Column(name: String, values: List(Float))
}

/// A handle to a column held elsewhere. Opaque: only this module can
/// make one, so an identifier cannot be invented by a caller.
pub opaque type Handle {
  Handle(id: Int)
}

/// What can go wrong.
pub type Error {
  Empty
  NotANumber(String)
  NoSuchColumn(Handle)
}

/// The mean and the deviation together.
pub type Summary =
  #(Float, Float)

const default_places = 3

const separator = ","

/// The arithmetic mean of a column.
pub fn mean(column: Column) -> Result(Float, Error) {
  case column.values {
    [] -> Error(Empty)
    values -> Ok(sum(values) /. count_of(values))
  }
}

/// The population standard deviation.
pub fn deviation(column: Column) -> Result(Float, Error) {
  use average <- result.try(mean(column))
  let squares =
    column.values
    |> list.map(fn(value) { { value -. average } *. { value -. average } })
  Ok(square_root(sum(squares) /. count_of(column.values)))
}

/// Both figures at once.
pub fn summarise(column: Column) -> Result(Summary, Error) {
  use average <- result.try(mean(column))
  use spread <- result.try(deviation(column))
  Ok(#(average, spread))
}

/// A one-line description. No return type, deliberately.
pub fn describe(column: Column) {
  string.concat([
    column.name,
    " (",
    int.to_string(list.length(column.values)),
    " readings)",
  ])
}

/// Reads a line into a column. Also missing its return type.
pub fn parse_line(name: String, line: String) {
  line
  |> string.split(separator)
  |> list.map(float.parse)
  |> result.all
  |> result.map(fn(values) { Column(name, values) })
  |> result.map_error(fn(_) { NotANumber(line) })
}

fn sum(values: List(Float)) -> Float {
  list.fold(values, 0.0, float.add)
}

fn count_of(values: List(Float)) -> Float {
  int.to_float(list.length(values))
}

fn rounded(value: Float) -> Float {
  float.to_precision(value, default_places)
}

@external(erlang, "math", "sqrt")
@external(javascript, "./maths.mjs", "squareRoot")
fn square_root(value: Float) -> Float
