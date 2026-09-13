# CMakeLists.txt

The build for csvstats, written the way CMake is written now: targets
with properties, rather than directory-wide variables.

It declares a project with a version and two languages, three options a
person can turn on or off at configure time, two packages it goes
looking for - one of them only when an option is on - a static library,
an executable, two tests, and three subdirectories it descends into.

The `option()` calls are the interesting part to a reader: they are the
knobs, and their defaults are what happens if nobody touches them.
