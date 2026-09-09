# Package

version       = "0.3.0"
author        = "Example"
description   = "Reads a comma-separated file and describes its columns."
license       = "MIT"
srcDir        = "src"
bin           = @["csvstats"]

# Dependencies

requires "nim >= 2.2.0"

# Tasks

task test, "Runs the test suite":
  exec "nim c -r --hints:off tests/test_csvstats.nim"

task lint, "Checks style without building":
  exec "nim check --styleCheck:error --hints:off src/csvstats.nim"
