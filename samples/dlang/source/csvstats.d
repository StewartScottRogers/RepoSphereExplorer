/**
 * Summary statistics for a column of a comma-separated file.
 */
module csvstats;

import std.algorithm : sum, map, filter;
import std.array : array, split;
import std.conv : to;
import std.math : sqrt;
import std.stdio : File;

/// Thrown when a column holds nothing to average.
class EmptyColumnException : Exception
{
    this(string name)
    {
        super("column " ~ name ~ " is empty");
    }
}

/// Gives whatever it is mixed into a name.
mixin template Named()
{
    string name;
}

/// A named column of numbers.
struct Column
{
    mixin Named;

    double[] values;

    /// The arithmetic mean.
    double mean() @safe pure const
    in (values.length > 0, "an empty column has no mean")
    {
        return values.sum / values.length;
    }

    /// The population standard deviation.
    double deviation() @safe pure const
    {
        const average = mean();
        double total = 0;
        foreach (value; values)
        {
            total += (value - average) ^^ 2;
        }
        return sqrt(total / values.length);
    }
}

/// The largest of `values`, whatever they are made of.
T largest(T)(T[] values) @safe pure nothrow
{
    T best = values[0];
    foreach (value; values)
    {
        if (value > best)
        {
            best = value;
        }
    }
    return best;
}

/// A template that reports whether `T` can be averaged at all.
template isAverageable(T)
{
    enum isAverageable = __traits(compiles, T.init + T.init);
}

/// Reads a file into one column per heading.
class Reader
{
    private string path;

    this(string path)
    {
        this.path = path;
    }

    /// Reads the file. Touches the filesystem, so it carries no
    /// safety attribute and is @system by default.
    Column[] read()
    {
        auto file = File(path, "r");
        auto headings = file.readln().split(",");
        Column[] columns;
        foreach (heading; headings)
        {
            Column column;
            column.name = heading;
            columns ~= column;
        }
        foreach (line; file.byLine())
        {
            auto fields = line.idup.split(",");
            foreach (index, field; fields)
            {
                columns[index].values ~= field.to!double;
            }
        }
        return columns;
    }
}

unittest
{
    Column column;
    column.name = "flat";
    column.values = [7, 7, 7];
    assert(column.mean() == 7);
    assert(column.deviation() == 0);
}

unittest
{
    assert(largest([1, 9, 4]) == 9);
    assert(largest([1.5, 2.5]) == 2.5);
    assert(isAverageable!double);
}
