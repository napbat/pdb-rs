' The VB demo uses Async, Iterator, Static locals and local constants. Without arguments, it
' prints a checksum and exits with code 0. With the argument "stack", it prints a stack trace.
Imports System
Imports System.Collections.Generic
Imports System.Threading.Tasks
Imports Gen = System.Collections.Generic
Imports <xmlns:demo="urn:pdbrs-demo">

Module Program

    Function Main(args As String()) As Integer
        If args.Length > 0 AndAlso args(0) = "stack" Then
            Tracing.TraceAsync(2).GetAwaiter().GetResult()
            Return 0
        End If

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
        Dim constants As Integer = LocalConstants(3)
        Dim stepped As Integer = New StepCounter(2).NextValue() + MappedRegion.Compute(0)

        Dim total As Integer = iteratorSum + counted + asyncResult + addOffset(1) + constants + stepped
        Console.WriteLine("vb demo checksum " & total.ToString())
        Return 0
    End Function

    Iterator Function Evens(count As Integer) As IEnumerable(Of Integer)
        For index As Integer = 0 To count - 1
            Yield index * 2
        Next
    End Function

    ' The stack mode calls this iterator from an async function in Tracing.vb.
    Iterator Function Traced(depth As Integer) As IEnumerable(Of Integer)
        For index As Integer = 0 To depth - 1
            If index = depth - 1 Then
                Yield Tracing.PrintTrace()
            Else
                Yield index
            End If
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

    ' Local constants and nested scopes in a function without a state machine.
    Function LocalConstants(seed As Integer) As Integer
        Const Limit As Integer = 3
        Const Wide As Long = 1L << 40
        Const Price As Decimal = 12.75D
        Const Stamp As Date = #2024-01-02#
        Const Text As String = "locals"
        Const Day As DayOfWeek = DayOfWeek.Friday
        Dim values As Gen.List(Of Integer) = New Gen.List(Of Integer)()
        For outer As Integer = 0 To Limit - 1
            Const StepSize As Integer = 2
            Dim doubled As Integer = outer * StepSize
            If doubled >= 0 Then
                Dim shifted As Integer = doubled + Text.Length
                values.Add(shifted)
            End If
        Next
        Dim total As Integer = seed + CInt(Wide >> 40) + CInt(Price) + Stamp.Day + CInt(Day)
        For Each value As Integer In values
            total += value
        Next
        Dim element As XElement = <demo:item>vb</demo:item>
        Return total + element.Value.Length + ConstantValues()
    End Function

    ' Local constants of each constant type of Visual Basic. A Windows PDB keeps each value in an
    ' S_MANCONSTANT record with a numeric leaf. The function returns 0, so the checksum of the
    ' demo does not change.
    Function ConstantValues() As Integer
        Const Earliest As Date = #1/1/0001#
        Const Stamp As Date = #2024-01-02 13:45:30#
        Const Latest As Date = #12/31/9999 11:59:59 PM#
        Const TimeOnly As Date = #6:30:00 AM#

        Const DecimalZero As Decimal = 0D
        Const DecimalScaled As Decimal = 12.750D
        Const DecimalNegative As Decimal = -0.0001D
        Const DecimalMax As Decimal = Decimal.MaxValue

        Const SingleValue As Single = 1.5F
        Const SingleNaN As Single = Single.NaN
        Const DoubleValue As Double = 0.1R
        Const DoubleNaN As Double = Double.NaN
        Const DoubleInfinity As Double = Double.NegativeInfinity
        Const DoubleNegativeZero As Double = -0.0R

        Const EmptyText As String = ""
        Const AsciiText As String = "constant"
        Const OtherText As String = "Grüße € 日本語 😀"
        Const NulText As String = "a" & vbNullChar & "b"

        Const Letter As Char = "A"c
        Const CharZero As Char = ChrW(0)
        Const CharEuro As Char = "€"c
        Const CharMax As Char = ChrW(&HFFFF)

        Const IsTrue As Boolean = True
        Const IsFalse As Boolean = False

        Const SByteMin As SByte = SByte.MinValue
        Const ByteMax As Byte = Byte.MaxValue
        Const ShortMin As Short = Short.MinValue
        Const UShortMax As UShort = UShort.MaxValue
        Const IntegerMin As Integer = Integer.MinValue
        Const UIntegerMax As UInteger = UInteger.MaxValue
        Const LongMin As Long = Long.MinValue
        Const ULongMax As ULong = ULong.MaxValue

        Const NoObject As Object = Nothing
        Const NoText As String = Nothing
        Const Day As DayOfWeek = DayOfWeek.Sunday

        Dim values As Object() = {
            Earliest, Stamp, Latest, TimeOnly, DecimalZero, DecimalScaled, DecimalNegative,
            DecimalMax, SingleValue, SingleNaN, DoubleValue, DoubleNaN, DoubleInfinity,
            DoubleNegativeZero, EmptyText, AsciiText, OtherText, NulText, Letter, CharZero,
            CharEuro, CharMax, IsTrue, IsFalse, SByteMin, ByteMax, ShortMin, UShortMax,
            IntegerMin, UIntegerMax, LongMin, ULongMax, NoObject, NoText, Day
        }
        Dim failures As Integer = If(values.Length = 35, 0, 1)
        failures += If(Double.IsNaN(DoubleNaN) AndAlso Single.IsNaN(SingleNaN), 0, 1)
        failures += If(Stamp.Hour = 13 AndAlso TimeOnly.Year = 1, 0, 1)
        Return failures
    End Function

End Module
