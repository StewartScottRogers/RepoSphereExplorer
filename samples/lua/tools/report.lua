-- A one-off script, written the way one-off scripts get written: every
-- name here is global, so requiring it from anywhere else would quietly
-- replace whatever else was using those names.

csvstats = require("csvstats")

path = arg[1] or "data.csv"
columns = csvstats.read(path)

for _, column in ipairs(columns) do
    mean = column:mean()
    if mean ~= nil then
        print(string.format("%-20s %10.3f %10.3f", column.name, mean, column:deviation()))
    end
end
