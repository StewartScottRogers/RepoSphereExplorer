# build.bat

Builds, tests and packages the Repos Explorer on Windows.

    build.bat
    build.bat dev

`:package` is never called and nothing jumps to it, so it runs only if
`:test` falls through into it. It is left that way so the file pane has
something to warn about.
