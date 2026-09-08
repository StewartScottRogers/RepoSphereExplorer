module Budget exposing
    ( Model
    , Msg(..)
    , Category(..)
    , Transaction
    , init
    , update
    , view
    , totalSpent
    , remaining
    )

{-| A small budget tracker in the Elm architecture: a model, messages that
describe what happened, an update that folds one into the other, and a view
that is a pure function of the result.
-}

import Browser
import Dict exposing (Dict)
import Html exposing (Html, button, div, input, li, span, text, ul)
import Html.Attributes exposing (class, placeholder, value)
import Html.Events exposing (onClick, onInput)


type Category
    = Rent
    | Food
    | Travel
    | Other String


type alias Transaction =
    { id : Int
    , description : String
    , pennies : Int
    , category : Category
    }


type alias Model =
    { transactions : List Transaction
    , budgetPennies : Int
    , draft : String
    , nextId : Int
    , error : Maybe String
    }


type Msg
    = DraftChanged String
    | Added Category
    | Removed Int
    | BudgetChanged String
    | Cleared


init : Int -> Model
init budgetPennies =
    { transactions = []
    , budgetPennies = budgetPennies
    , draft = ""
    , nextId = 1
    , error = Nothing
    }


categoryName : Category -> String
categoryName category =
    case category of
        Rent ->
            "Rent"

        Food ->
            "Food"

        Travel ->
            "Travel"

        Other label ->
            label


parsePennies : String -> Result String Int
parsePennies raw =
    case String.toFloat (String.trim raw) of
        Nothing ->
            Err ("not an amount: " ++ raw)

        Just pounds ->
            Ok (round (pounds * 100))


totalSpent : Model -> Int
totalSpent model =
    List.sum (List.map .pennies model.transactions)


remaining : Model -> Int
remaining model =
    model.budgetPennies - totalSpent model


byCategory : Model -> Dict String Int
byCategory model =
    List.foldl
        (\transaction acc ->
            Dict.update (categoryName transaction.category)
                (\existing -> Just (Maybe.withDefault 0 existing + transaction.pennies))
                acc
        )
        Dict.empty
        model.transactions


update : Msg -> Model -> Model
update msg model =
    case msg of
        DraftChanged draft ->
            { model | draft = draft, error = Nothing }

        Added category ->
            case parsePennies model.draft of
                Err message ->
                    { model | error = Just message }

                Ok pennies ->
                    { model
                        | transactions =
                            { id = model.nextId
                            , description = categoryName category
                            , pennies = pennies
                            , category = category
                            }
                                :: model.transactions
                        , nextId = model.nextId + 1
                        , draft = ""
                        , error = Nothing
                    }

        Removed id ->
            { model | transactions = List.filter (\t -> t.id /= id) model.transactions }

        BudgetChanged raw ->
            case parsePennies raw of
                Err message ->
                    { model | error = Just message }

                Ok pennies ->
                    { model | budgetPennies = pennies }

        Cleared ->
            { model | transactions = [], error = Nothing }


formatPennies : Int -> String
formatPennies pennies =
    String.fromFloat (toFloat pennies / 100)


viewTransaction : Transaction -> Html Msg
viewTransaction transaction =
    li [ class "transaction" ]
        [ span [ class "label" ] [ text transaction.description ]
        , span [ class "amount" ] [ text (formatPennies transaction.pennies) ]
        , button [ onClick (Removed transaction.id) ] [ text "remove" ]
        ]


view : Model -> Html Msg
view model =
    div [ class "budget" ]
        [ input [ placeholder "amount", value model.draft, onInput DraftChanged ] []
        , button [ onClick (Added Food) ] [ text "add food" ]
        , button [ onClick (Added Travel) ] [ text "add travel" ]
        , button [ onClick Cleared ] [ text "clear" ]
        , ul [] (List.map viewTransaction model.transactions)
        , div [ class "remaining" ] [ text ("remaining " ++ formatPennies (remaining model)) ]
        , case model.error of
            Nothing ->
                text ""

            Just message ->
                div [ class "error" ] [ text message ]
        ]


main : Program () Model Msg
main =
    Browser.sandbox { init = init 150000, update = update, view = view }
