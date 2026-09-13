# readings.cue

CUE does not separate a schema from the data it validates: both are
values, and checking one against the other is the same operation as
combining them.

`#Reading` and `#Column` are definitions - closed, so a field nobody
declared is an error rather than an extra. The constraints are written
inline: a temperature is `>=-90.0 & <=60.0`, a quality is one of three
strings, a unit is a disjunction with a default marked `*`. `note?` is
optional, and the `?` is the whole of that.

`column` at the bottom is a concrete instance, unified with `#Column`
by writing `&` between them. It is valid: every reading is inside the
range, which is the point of having the range written down.
