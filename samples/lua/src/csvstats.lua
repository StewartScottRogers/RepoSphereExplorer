--- Summary statistics for a column of a comma-separated file.
-- @module csvstats

local io = require("io")
local math = require("math")

local M = {}

local Column = {}
Column.__index = Column

--[[
An earlier version kept a running median here. It was wrong for an even
number of samples, and nothing used it:

function Column:median()
    return self.values[#self.values / 2]
end
]]

--- Makes an empty column.
-- @param name the column's heading
function Column.new(name)
    local self = setmetatable({}, Column)
    self.name = name
    self.values = {}
    return self
end

function Column:add(value)
    self.values[#self.values + 1] = value
end

function Column:mean()
    if #self.values == 0 then
        return nil
    end
    local total = 0
    for _, value in ipairs(self.values) do
        total = total + value
    end
    return total / #self.values
end

function Column:deviation()
    local mean = self:mean()
    if mean == nil then
        return nil
    end
    local sum = 0
    for _, value in ipairs(self.values) do
        sum = sum + (value - mean) ^ 2
    end
    return math.sqrt(sum / #self.values)
end

local function parse_line(line)
    local fields = {}
    for field in line:gmatch("[^,]+") do
        fields[#fields + 1] = field
    end
    return fields
end

--- Reads a file and returns one Column per heading.
function M.read(path)
    local handle = assert(io.open(path, "r"))
    local headings = parse_line(handle:read("l"))
    local columns = {}
    for _, heading in ipairs(headings) do
        columns[#columns + 1] = Column.new(heading)
    end
    for line in handle:lines() do
        local fields = parse_line(line)
        for index, field in ipairs(fields) do
            local number = tonumber(field)
            if number ~= nil then
                columns[index]:add(number)
            end
        end
    end
    handle:close()
    return columns
end

M.Column = Column

return M
