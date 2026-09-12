# csvstats

Summary statistics for a column of readings, in PureScript.

    spago build
    spago run --main Test.Main

`roundTo` in `src/Data/CsvStats.purs` has no type signature above it, and
`report` is defined but left out of the export list. Both are left that
way so the file pane has something to warn about, along with the two
`foreign import` declarations whose JavaScript the compiler never sees.
