#!/usr/bin/awk -f
#
# Summary statistics for a comma-separated file.
#
#   awk -f csvstats.awk samples.csv
#
# `carried` inside average() is not one of its parameters, so it is
# global and survives the call. It is left that way so the file pane has
# something to warn about.

BEGIN {
    FS = ","
    OFS = "\t"
    count = 0
    total = 0
    squares = 0
}

# Comment rows, and the heading, are not readings.
/^#/ { next }

NR == 1 {
    heading = $2
    next
}

# Anything with fewer than three fields is malformed; print it as it is
# so the caller can see what was dropped.
NF < 3

NF >= 3 {
    value = $2 + 0
    total += value
    squares += value * value
    seen[$1] = $3
    count++
}

function average(sum, n,    result) {
    if (n == 0) {
        return 0
    }
    result = sum / n
    carried = result
    return result
}

function spread(sum, sq, n,    mean) {
    if (n == 0) {
        return 0
    }
    mean = sum / n
    return sqrt(sq / n - mean * mean)
}

END {
    printf "%s: %d rows\n", heading, count
    printf "mean      %.3f\n", average(total, count)
    printf "deviation %.3f\n", spread(total, squares, count)
    for (key in seen) {
        printf "  %s %s\n", key, seen[key]
    }
}
