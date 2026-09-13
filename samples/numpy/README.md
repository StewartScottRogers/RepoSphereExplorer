# readings

`readings.npy` is a twenty-four by six array of doubles, laid out row by
row. `column-major.npy` holds the same numbers laid out the other way
round — a reader that ignores the Fortran-order flag gets a transposed
array and is never told.

`station.npz` is a zip archive of three arrays with three different data
types: identifiers as 32-bit integers, readings as 32-bit floats, and a
boolean mask.
