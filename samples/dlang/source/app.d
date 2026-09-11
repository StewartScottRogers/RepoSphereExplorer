module app;

import std.stdio : writefln;
import csvstats : Reader;

void main(string[] arguments) @system
{
    const path = arguments.length > 1 ? arguments[1] : "data.csv";
    auto reader = new Reader(path);

    foreach (column; reader.read())
    {
        if (column.values.length == 0)
        {
            continue;
        }
        writefln("%-20s %10.3f %10.3f", column.name, column.mean(), column.deviation());
    }
}
