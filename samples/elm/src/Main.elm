module Main exposing (main)

{-| The application shell. Everything worth testing lives in `Budget`, so
this is `Browser.sandbox` and nothing else - a shell with logic in it is a
shell nobody can test.
-}

import Browser
import Budget


main : Program () Budget.Model Budget.Msg
main =
    Browser.sandbox
        { init = Budget.init 250000
        , update = Budget.update
        , view = Budget.view
        }
