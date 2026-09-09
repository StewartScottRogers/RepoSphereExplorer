import std/[strutils, unittest]

import ../src/csvstats

const Sample = """
name,age,score
ada,36,91.5
grace,45,88.0
alan,41,79.25
"""

suite "parse":
  test "reads one column per header field":
    let columns = parse(Sample)

    check columns.len == 3

  test "keeps the header names":
    let columns = parse(Sample)

    check columns[0].name == "name"
    check columns[1].name == "age"

  test "an empty document yields no columns rather than failing":
    check parse("").len == 0

suite "detect":
  test "a column of integers is not mistaken for text":
    let kinds = detect(@["age"], @[@["36"], @["45"]])

    check kinds.len == 1

suite "summarise":
  test "a numeric column reports something about its values":
    let columns = parse(Sample)

    let summary = summarise(columns[1])

    check summary.count == 3

suite "report":
  test "the report names every column it was given":
    let text = report(parse(Sample))

    check "name" in text
    check "age" in text
