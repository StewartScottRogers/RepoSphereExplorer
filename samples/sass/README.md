# _readings.scss and legacy.sass

The same stylesheet in both of Sass's syntaxes, because a reader opening
one needs to be told which they are looking at.

`_readings.scss` is the braced syntax everybody writes now, and it is a
**partial**: the leading underscore means it is never compiled on its
own, only through whatever `@use`s it. It pulls in two built-in modules
and one of its own, forwards a third with a `show` list, and defines
variables, two functions, two mixins - one of them taking `@content` -
two placeholder selectors, and a rule nested five levels deep.

`legacy.sass` is the original indented syntax: no braces, no
semicolons, `=` for a mixin and `+` to include one. It is still
compiled by the same tool, and telling it from the other is not
something a file extension should have to do alone.
