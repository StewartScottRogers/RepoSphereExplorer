package com.example.csvstats;

import java.util.List;

/** One column of readings, and the statistics it can report. */
@Deprecated
public final class Column implements Readable {
    private static final int LIMIT = 1024;

    private final String name;

    public Column(String name) {
        this.name = name;
    }

    @Override
    public String name() {
        return this.name;
    }

    /** The mean of any readings that are numbers at all. */
    public <T extends Number> double mean(List<T> readings) {
        double total = 0.0;
        for (T reading : readings) {
            total += reading.doubleValue();
        }
        return readings.isEmpty() ? 0.0 : total / readings.size();
    }

    /** What a column reports once it has been read. */
    public static final class Summary {
        public double mean;
    }
}
