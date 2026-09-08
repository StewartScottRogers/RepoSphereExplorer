/// A tiny arithmetic expression parser and evaluator: discriminated
/// unions for the tree, active patterns for the lexer, and a recursive
/// descent parser returning Result rather than throwing.
module Parser

open System

type Token =
    | Number of float
    | Plus
    | Minus
    | Star
    | Slash
    | LeftParen
    | RightParen

type Expr =
    | Literal of float
    | Negate of Expr
    | Add of Expr * Expr
    | Subtract of Expr * Expr
    | Multiply of Expr * Expr
    | Divide of Expr * Expr

type ParseError =
    | UnexpectedCharacter of char * int
    | UnexpectedEnd
    | UnbalancedParenthesis of int
    | TrailingInput of Token list

let private isDigitOrDot c = Char.IsDigit c || c = '.'

let tokenize (input: string) : Result<Token list, ParseError> =
    let rec loop index acc =
        if index >= input.Length then
            Ok(List.rev acc)
        else
            match input.[index] with
            | c when Char.IsWhiteSpace c -> loop (index + 1) acc
            | '+' -> loop (index + 1) (Plus :: acc)
            | '-' -> loop (index + 1) (Minus :: acc)
            | '*' -> loop (index + 1) (Star :: acc)
            | '/' -> loop (index + 1) (Slash :: acc)
            | '(' -> loop (index + 1) (LeftParen :: acc)
            | ')' -> loop (index + 1) (RightParen :: acc)
            | c when Char.IsDigit c ->
                let literal =
                    input.[index..]
                    |> Seq.takeWhile isDigitOrDot
                    |> Seq.toArray
                    |> String

                loop (index + literal.Length) (Number(Double.Parse literal) :: acc)
            | other -> Error(UnexpectedCharacter(other, index))

    loop 0 []

let rec private parseExpression tokens =
    parseTerm tokens
    |> Result.bind (fun (left, rest) ->
        let rec fold left rest =
            match rest with
            | Plus :: tail ->
                parseTerm tail |> Result.bind (fun (right, tail') -> fold (Add(left, right)) tail')
            | Minus :: tail ->
                parseTerm tail
                |> Result.bind (fun (right, tail') -> fold (Subtract(left, right)) tail')
            | _ -> Ok(left, rest)

        fold left rest)

and private parseTerm tokens =
    parseFactor tokens
    |> Result.bind (fun (left, rest) ->
        let rec fold left rest =
            match rest with
            | Star :: tail ->
                parseFactor tail
                |> Result.bind (fun (right, tail') -> fold (Multiply(left, right)) tail')
            | Slash :: tail ->
                parseFactor tail
                |> Result.bind (fun (right, tail') -> fold (Divide(left, right)) tail')
            | _ -> Ok(left, rest)

        fold left rest)

and private parseFactor tokens =
    match tokens with
    | Number value :: rest -> Ok(Literal value, rest)
    | Minus :: rest -> parseFactor rest |> Result.map (fun (inner, tail) -> Negate inner, tail)
    | LeftParen :: rest ->
        parseExpression rest
        |> Result.bind (fun (inner, tail) ->
            match tail with
            | RightParen :: after -> Ok(inner, after)
            | _ -> Error(UnbalancedParenthesis(List.length rest)))
    | [] -> Error UnexpectedEnd
    | other -> Error(TrailingInput other)

let parse (input: string) : Result<Expr, ParseError> =
    tokenize input
    |> Result.bind parseExpression
    |> Result.bind (fun (expr, rest) ->
        match rest with
        | [] -> Ok expr
        | leftovers -> Error(TrailingInput leftovers))

let rec evaluate expr =
    match expr with
    | Literal value -> value
    | Negate inner -> -(evaluate inner)
    | Add(left, right) -> evaluate left + evaluate right
    | Subtract(left, right) -> evaluate left - evaluate right
    | Multiply(left, right) -> evaluate left * evaluate right
    | Divide(left, right) -> evaluate left / evaluate right

let rec render expr =
    match expr with
    | Literal value -> string value
    | Negate inner -> sprintf "-%s" (render inner)
    | Add(l, r) -> sprintf "(%s + %s)" (render l) (render r)
    | Subtract(l, r) -> sprintf "(%s - %s)" (render l) (render r)
    | Multiply(l, r) -> sprintf "(%s * %s)" (render l) (render r)
    | Divide(l, r) -> sprintf "(%s / %s)" (render l) (render r)

let describe input =
    match parse input with
    | Ok expr -> sprintf "%s = %g" (render expr) (evaluate expr)
    | Error err -> sprintf "could not parse %s: %A" input err

[<EntryPoint>]
let main argv =
    let inputs =
        if Array.isEmpty argv then
            [| "1 + 2 * 3"; "(4 + 5) / 3"; "-7 + 2"; "2 +" |]
        else
            argv

    inputs |> Array.iter (describe >> printfn "%s")
    0
