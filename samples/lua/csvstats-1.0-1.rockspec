package = "csvstats"
version = "1.0-1"

source = {
   url = "git+https://example.com/floor/csvstats.git",
   tag = "v1.0"
}

description = {
   summary = "Summary statistics for comma-separated files.",
   detailed = [[
      Reads a comma-separated file and reports the mean and standard
      deviation of every column that holds numbers.
   ]],
   homepage = "https://example.com/floor/csvstats",
   license = "MIT"
}

dependencies = {
   "lua >= 5.3",
   "luafilesystem >= 1.8"
}

build = {
   type = "builtin",
   modules = {
      csvstats = "src/csvstats.lua"
   }
}
