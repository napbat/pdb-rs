// The first F# source file. It has the output of the stack mode, an abstract member and an
// extern function. The abstract member and the extern function have no sequence points.
module PdbrsDemo.Tracing

open System
open System.Diagnostics
open System.Runtime.CompilerServices
open System.Runtime.InteropServices

/// Print the stack trace with file names and line numbers. The optimized build must not inline
/// this function, so that its frame stays in the stack trace.
[<MethodImpl(MethodImplOptions.NoInlining)>]
let printTrace () =
    let trace = StackTrace(true)
    Console.Write(trace.ToString())
    0

[<AbstractClass>]
type Counter() =
    abstract Next: unit -> int

type StepCounter(step: int) =
    inherit Counter()
    let mutable value = 0
    override _.Next() =
        value <- value + step
        value

module NativeMethods =
    // The demo never calls this function. It gives a method without a body.
    [<DllImport("kernel32.dll")>]
    extern uint32 GetCurrentProcessId()
