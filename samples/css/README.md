# readings.css

A stylesheet for the readings table, written the way one is now.

Custom properties on `:root` and read back with `var()`; a `@font-face`
with two sources and a variable weight range; nesting with `&`, which no
preprocessor is doing here because the browser does it; a container
query beside the media queries, because the panel cares how wide *it*
is rather than how wide the window is; `@supports`; and a keyframe
animation that a `prefers-reduced-motion` query turns back off.

Colours are deliberately written four ways - hex, `rgb()`, `hsl()` and
named - because a reader asking "what colours does this use" is asking
across all of them.
