"""Summary statistics for a column of readings."""

LIMIT = 1024


def mean(readings, default=0.0):
    """The mean of the readings, or `default` when there are none."""
    total = 0.0
    for reading in readings:
        total += float(reading)
    return total / len(readings) if readings else default
