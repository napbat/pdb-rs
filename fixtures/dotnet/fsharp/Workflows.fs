// The second F# source file. It has sequence expressions (iterators) and task expressions (async
// state machines).
module PdbrsDemo.Workflows

open System.Threading.Tasks

let evens count =
    seq {
        for index in 0 .. count - 1 do
            yield index * 2
    }

/// The stack mode calls this sequence from a task expression.
let traced depth =
    seq {
        for index in 0 .. depth - 1 do
            if index = depth - 1 then
                yield Tracing.printTrace ()
            else
                yield index
    }

let computeTask seed =
    task {
        let captured = seed * 2
        do! Task.Yield()
        do! Task.Delay 1
        return captured + 1
    }

/// The stack mode enumerates a sequence before the first wait.
let traceTask depth =
    task {
        let mutable sum = 0
        for value in traced depth do
            sum <- sum + value
        do! Task.Yield()
        return sum
    }
