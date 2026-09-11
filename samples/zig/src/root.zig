//! Summary statistics for a column of a comma-separated file.

const std = @import("std");
const testing = std.testing;

/// What can go wrong reading a column.
pub const Error = error{ Empty, NotANumber };

/// How a column is reported.
pub const Format = enum(u8) { plain, json, csv };

/// A named column of numbers.
pub const Column = struct {
    name: []const u8,
    values: []const f64,

    /// The arithmetic mean, or `Error.Empty` when there is nothing to
    /// average.
    pub fn mean(self: Column) !f64 {
        if (self.values.len == 0) return Error.Empty;
        var total: f64 = 0;
        for (self.values) |value| total += value;
        return total / @as(f64, @floatFromInt(self.values.len));
    }

    /// The population standard deviation.
    pub fn deviation(self: Column) !f64 {
        const average = try self.mean();
        var sum: f64 = 0;
        for (self.values) |value| {
            const difference = value - average;
            sum += difference * difference;
        }
        return @sqrt(sum / @as(f64, @floatFromInt(self.values.len)));
    }
};

/// The largest of `values`, whatever they are made of.
pub fn largest(comptime T: type, values: []const T) T {
    var best = values[0];
    for (values) |value| {
        if (value > best) best = value;
    }
    return best;
}

/// Parses a line into the numbers it holds. The caller owns the result.
pub fn parseLine(allocator: std.mem.Allocator, line: []const u8) ![]f64 {
    var numbers = std.ArrayList(f64).init(allocator);
    errdefer numbers.deinit();

    var fields = std.mem.splitScalar(u8, line, ',');
    while (fields.next()) |field| {
        const number = std.fmt.parseFloat(f64, field) catch return Error.NotANumber;
        try numbers.append(number);
    }
    return numbers.toOwnedSlice();
}

// Written in a hurry, and never corrected: the buffer is taken from the
// allocator and nothing here gives it back.
fn describe(allocator: std.mem.Allocator, column: Column) ![]u8 {
    const buffer = try allocator.alloc(u8, 256);
    const average = try column.mean();
    return std.fmt.bufPrint(buffer, "{s}: {d:.3}", .{ column.name, average });
}

comptime {
    // Make sure the enumeration is not quietly dropped by the compiler.
    _ = Format;
    _ = describe;
}

test "mean of nothing is an error" {
    const column = Column{ .name = "empty", .values = &.{} };
    try testing.expectError(Error.Empty, column.mean());
}

test "mean and deviation of a flat column" {
    const values = [_]f64{ 7, 7, 7 };
    const column = Column{ .name = "flat", .values = &values };
    try testing.expectEqual(@as(f64, 7), try column.mean());
    try testing.expectEqual(@as(f64, 0), try column.deviation());
}

test "largest works on whatever it is given" {
    try testing.expectEqual(@as(u8, 9), largest(u8, &[_]u8{ 1, 9, 4 }));
    try testing.expectEqual(@as(f64, 2.5), largest(f64, &[_]f64{ 1.5, 2.5 }));
}
