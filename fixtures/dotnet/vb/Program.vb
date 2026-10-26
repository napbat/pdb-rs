' The VB demo uses Async, Iterator and Static locals. It exits with code 0.
Imports System
Imports System.Collections.Generic
Imports System.Threading.Tasks

Module Program

    Function Main() As Integer
        Dim iteratorSum As Integer = 0
        For Each value As Integer In Evens(5)
            iteratorSum += value
        Next

        Dim counted As Integer = 0
        For index As Integer = 1 To 3
            counted = NextCount()
        Next

        Dim asyncResult As Integer = ComputeAsync(7).GetAwaiter().GetResult()
        Dim offset As Integer = 2
        Dim addOffset As Func(Of Integer, Integer) = Function(value) value + offset

        Console.WriteLine("vb demo checksum " & (iteratorSum + counted + asyncResult + addOffset(1)).ToString())
        Return 0
    End Function

    Iterator Function Evens(count As Integer) As IEnumerable(Of Integer)
        For index As Integer = 0 To count - 1
            Yield index * 2
        Next
    End Function

    Function NextCount() As Integer
        Static calls As Integer = 0
        calls += 1
        Return calls
    End Function

    Async Function ComputeAsync(seed As Integer) As Task(Of Integer)
        Dim captured As Integer = seed * 2
        Await Task.Yield()
        Await Task.Delay(1)
        Return captured + 1
    End Function

End Module
