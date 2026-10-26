' The second VB source file. It has the async part and the output of the stack mode, an abstract
' function and a Declare function. The abstract and Declare functions have no sequence points.
Imports System
Imports System.Diagnostics
Imports System.Threading.Tasks

Module Tracing

    ' The stack mode enumerates an iterator in Program.vb before the first Await.
    Async Function TraceAsync(depth As Integer) As Task(Of Integer)
        Dim sum As Integer = 0
        For Each value As Integer In Program.Traced(depth)
            sum += value
        Next
        Await Task.Yield()
        Return sum
    End Function

    ' Print the stack trace with file names and line numbers.
    Function PrintTrace() As Integer
        Dim trace As New StackTrace(True)
        Console.Write(trace.ToString())
        Return 0
    End Function

    ' The demo never calls this function. It gives a method without a body.
    Declare Function GetCurrentProcessId Lib "kernel32.dll" () As UInteger

End Module

MustInherit Class Counter
    MustOverride Function NextValue() As Integer
End Class

NotInheritable Class StepCounter
    Inherits Counter

    Private ReadOnly stepSize As Integer
    Private value As Integer

    Sub New(stepSize As Integer)
        Me.stepSize = stepSize
    End Sub

    Overrides Function NextValue() As Integer
        value += stepSize
        Return value
    End Function

End Class
