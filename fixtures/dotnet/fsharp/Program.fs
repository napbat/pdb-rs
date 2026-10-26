// The F# demo uses an async workflow, task and sequence expressions, records, discriminated unions
// and pattern matching. Without arguments, it prints a checksum and exits with code 0. With the
// argument "stack", it prints a stack trace with file names and line numbers.
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

let checksum () =
    let origin = { X = 1; Y = 2 }
    let moved = { origin with Y = 5 }
    let shapes = [ Circle 2.0; Rect (3.0, 4.0); Empty ]
    let total = shapes |> List.sumBy area
    let asyncResult = computeAsync 7 |> Async.RunSynchronously
    let taskResult = (Workflows.computeTask 7).Result
    let evenSum = Workflows.evens 5 |> Seq.sum
    let stepped = Tracing.StepCounter(2).Next()
    let sum = int total + moved.X + moved.Y + asyncResult + taskResult + evenSum + stepped
    printfn "fsharp demo checksum %d" sum

[<EntryPoint>]
let main argv =
    if argv.Length > 0 && argv.[0] = "stack" then
        (Workflows.traceTask 2).Result |> ignore
    else
        checksum ()
    0
