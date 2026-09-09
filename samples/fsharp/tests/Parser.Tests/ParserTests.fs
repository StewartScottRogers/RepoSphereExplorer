module Parser.Tests

open Parser
open Xunit

[<Fact>]
let ``tokenize reads a number`` () =
    match tokenize "42" with
    | Ok tokens -> Assert.NotEmpty tokens
    | Error err -> failwithf "expected tokens, got %A" err

[<Fact>]
let ``tokenize refuses a character it does not know`` () =
    match tokenize "1 ? 2" with
    | Ok _ -> failwith "a character with no token should not tokenize"
    | Error _ -> ()

[<Fact>]
let ``parse reads an expression`` () =
    match parse "1 + 2 * 3" with
    | Ok expr -> Assert.NotNull(box expr)
    | Error err -> failwithf "expected an expression, got %A" err

[<Fact>]
let ``parse refuses an expression that ends early`` () =
    match parse "1 +" with
    | Ok _ -> failwith "a dangling operator should not parse"
    | Error _ -> ()

[<Fact>]
let ``an empty input is an error, not an empty expression`` () =
    match parse "" with
    | Ok _ -> failwith "nothing is not an expression"
    | Error _ -> ()
