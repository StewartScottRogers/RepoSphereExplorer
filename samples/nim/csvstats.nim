## Reads a CSV of numeric columns and reports per-column statistics.
##
## Exercises the shapes a Nim preview should show: types, an enum, an
## object variant, procs, funcs, methods, templates and an iterator.

import std/[algorithm, math, strformat, strutils, tables]

type
  ColumnKind* = enum
    ckNumeric, ckText, ckEmpty

  Column* = object
    name*: string
    case kind*: ColumnKind
    of ckNumeric:
      values*: seq[float]
    of ckText:
      texts*: seq[string]
    of ckEmpty:
      discard

  Summary* = object
    name*: string
    count*: int
    mean*: float
    median*: float
    stdev*: float
    minimum*: float
    maximum*: float

  ParseError* = object of CatchableError

template requireRow(condition: bool, message: string) =
  if not condition:
    raise newException(ParseError, message)

func isNumeric(cell: string): bool =
  try:
    discard parseFloat(cell.strip())
    true
  except ValueError:
    false

iterator rows*(text: string): seq[string] =
  for line in text.splitLines():
    if line.strip().len > 0:
      yield line.split(',').mapIt(it.strip())

proc detect*(header: seq[string], sample: seq[seq[string]]): seq[ColumnKind] =
  result = newSeq[ColumnKind](header.len)
  for index in 0 ..< header.len:
    var numeric = 0
    var seen = 0
    for row in sample:
      if index < row.len and row[index].len > 0:
        inc seen
        if isNumeric(row[index]):
          inc numeric
    result[index] =
      if seen == 0: ckEmpty
      elif numeric == seen: ckNumeric
      else: ckText

proc parse*(text: string): seq[Column] =
  var iterator_rows: seq[seq[string]]
  for row in rows(text):
    iterator_rows.add(row)

  requireRow(iterator_rows.len >= 2, "a CSV needs a header and at least one row")

  let header = iterator_rows[0]
  let body = iterator_rows[1 .. ^1]
  let kinds = detect(header, body)

  for index, name in header:
    case kinds[index]
    of ckNumeric:
      var column = Column(name: name, kind: ckNumeric)
      for row in body:
        if index < row.len and row[index].len > 0:
          column.values.add(parseFloat(row[index]))
      result.add(column)
    of ckText:
      var column = Column(name: name, kind: ckText)
      for row in body:
        if index < row.len:
          column.texts.add(row[index])
      result.add(column)
    of ckEmpty:
      result.add(Column(name: name, kind: ckEmpty))

func median(values: seq[float]): float =
  if values.len == 0:
    return 0.0
  var sorted = values
  sorted.sort()
  if sorted.len mod 2 == 1:
    sorted[sorted.len div 2]
  else:
    (sorted[sorted.len div 2 - 1] + sorted[sorted.len div 2]) / 2

proc summarise*(column: Column): Summary =
  requireRow(column.kind == ckNumeric, &"{column.name} is not numeric")

  let values = column.values
  let total = values.foldl(a + b, 0.0)
  let mean = total / values.len.float
  let variance = values.foldl(a + (b - mean) ^ 2, 0.0) / values.len.float

  Summary(
    name: column.name,
    count: values.len,
    mean: mean,
    median: median(values),
    stdev: sqrt(variance),
    minimum: values.min,
    maximum: values.max,
  )

proc report*(columns: seq[Column]): string =
  var lines = @[&"""{"column":<10}{"n":>5}{"mean":>10}{"median":>10}{"stdev":>10}"""]
  for column in columns:
    if column.kind == ckNumeric:
      let stats = summarise(column)
      lines.add(&"{stats.name:<10}{stats.count:>5}{stats.mean:>10.3f}" &
                &"{stats.median:>10.3f}{stats.stdev:>10.3f}")
  lines.join("\n")

when isMainModule:
  const sample = """
station,celsius,humidity
LHR,14.5,71
JFK,19.2,55
NRT,22.8,64
SYD,25.1,49
"""
  echo report(parse(sample))
