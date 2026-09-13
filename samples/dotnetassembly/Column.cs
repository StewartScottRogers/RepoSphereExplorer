namespace CsvStats;

/// <summary>How a column's readings are summarised.</summary>
public enum Measure
{
    /// <summary>The arithmetic mean.</summary>
    Mean,
    /// <summary>The middle reading.</summary>
    Median,
}

/// <summary>Something that yields readings.</summary>
public interface IReader
{
    /// <summary>Every reading, in file order.</summary>
    IEnumerable<double> Read(string path);
}

/// <summary>What a column reports once it has been read.</summary>
public readonly record struct Summary(double Value, int Count, Measure Measure);

/// <summary>One column of readings.</summary>
public sealed class Column
{
    private readonly List<double> readings = new();

    /// <summary>The column's name, as the header row spells it.</summary>
    public string Name { get; }

    /// <summary>Names the column.</summary>
    public Column(string name) => Name = name;

    /// <summary>Adds one reading.</summary>
    public void Add(double reading) => readings.Add(reading);

    /// <summary>Summarises the readings so far.</summary>
    public Summary Summarise(Measure measure)
    {
        if (readings.Count == 0)
        {
            return new Summary(0.0, 0, measure);
        }

        if (measure == Measure.Mean)
        {
            return new Summary(readings.Sum() / readings.Count, readings.Count, measure);
        }

        var sorted = readings.OrderBy(one => one).ToArray();
        return new Summary(sorted[sorted.Length / 2], sorted.Length, measure);
    }
}
