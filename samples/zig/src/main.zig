const std = @import("std");
const csvstats = @import("root.zig");

pub fn main() !void {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const allocator = arena.allocator();

    const arguments = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, arguments);

    const path = if (arguments.len > 1) arguments[1] else "data.csv";
    const file = try std.fs.cwd().openFile(path, .{});
    defer file.close();

    const stdout = std.io.getStdOut().writer();
    try stdout.print("reading {s}\n", .{path});

    var buffer: [4096]u8 = undefined;
    const read = try file.readAll(&buffer);
    const numbers = try csvstats.parseLine(allocator, buffer[0..read]);
    defer allocator.free(numbers);

    const column = csvstats.Column{ .name = path, .values = numbers };
    try stdout.print("mean {d:.3}\n", .{try column.mean()});
}
