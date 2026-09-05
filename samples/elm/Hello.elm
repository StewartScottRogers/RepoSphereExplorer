module Hello exposing (main, greeting)

type alias Greeting =
    { name : String
    , message : String
    }

greeting : Greeting
greeting =
    { name = "world"
    , message = "Hello"
    }

main : String
main =
    greeting.message ++ ", " ++ greeting.name ++ "!"
