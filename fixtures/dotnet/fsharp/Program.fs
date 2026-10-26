// The F# demo uses an async workflow, records, discriminated unions and pattern matching.
module PdbrsDemo.Program

type Point = { X: int; Y: int }

type Shape =
    | Circle of radius: float
    | Rect of width: float * height: float
    | Empty

let area shape =
    match shape with
    | Circle radius -> 3.25 * radius * radius
    | Rect (width, height) when width = height -> width * width
    | Rect (width, height) -> width * height
    | Empty -> 0.0

let computeAsync seed =
    async {
        let captured = seed * 2
        do! Async.Sleep 1
        return captured + 1
    }

[<EntryPoint>]
let main _ =
    let origin = { X = 1; Y = 2 }
    let moved = { origin with Y = 5 }
    let shapes = [ Circle 2.0; Rect (3.0, 4.0); Empty ]
    let total = shapes |> List.sumBy area
    let asyncResult = computeAsync 7 |> Async.RunSynchronously
    printfn "fsharp demo checksum %d" (int total + moved.X + moved.Y + asyncResult)
    0
