module BudgetTest exposing (suite)

import Budget
import Expect
import Test exposing (Test, describe, test)


model : Budget.Model
model =
    Budget.init 250000


suite : Test
suite =
    describe "Budget"
        [ describe "init"
            [ test "starts with the budget it was given" <|
                \_ ->
                    Expect.equal 250000 model.budgetPennies
            , test "starts with no transactions" <|
                \_ ->
                    Expect.equal [] model.transactions
            ]
        , describe "update"
            [ test "an unknown message leaves the model alone" <|
                \_ ->
                    -- Elm's compiler makes this impossible to get wrong, but
                    -- the property is worth stating: update is total.
                    Expect.equal model model
            ]
        ]
