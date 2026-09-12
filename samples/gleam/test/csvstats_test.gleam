import gleam/should
import gleeunit
import csvstats.{Column, Empty}

pub fn main() {
  gleeunit.main()
}

pub fn mean_of_nothing_test() {
  Column("empty", [])
  |> csvstats.mean
  |> should.equal(Error(Empty))
}

pub fn mean_of_three_test() {
  Column("n", [1.0, 2.0, 3.0])
  |> csvstats.mean
  |> should.equal(Ok(2.0))
}

pub fn describe_test() {
  Column("n", [1.0, 2.0])
  |> csvstats.describe
  |> should.equal("n (2 readings)")
}
