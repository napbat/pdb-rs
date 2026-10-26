// Local constants of each constant type of C#. A Windows PDB keeps each value in an
// S_MANCONSTANT record with a numeric leaf; a Portable PDB keeps it in a local constant
// signature. Check returns 0, so the checksum of the demo does not change.
using System;

namespace PdbrsDemo;

internal enum Level : sbyte
{
    Low = -128,
    High = 127,
}

internal enum Mask : ulong
{
    None = 0,
    All = ulong.MaxValue,
}

internal enum Offset : long
{
    Back = long.MinValue,
}

internal static class ConstantValues
{
    public static int Check()
    {
        const float Single = 1.5f;
        const float SingleNegative = -0.25f;
        const float SingleMax = float.MaxValue;
        const float SingleEpsilon = float.Epsilon;
        const float SingleNaN = float.NaN;
        const float SingleInfinity = float.PositiveInfinity;
        const float SingleNegativeZero = -0.0f;

        const double Double = 0.1;
        const double DoubleMin = double.MinValue;
        const double DoubleEpsilon = double.Epsilon;
        const double DoubleNaN = double.NaN;
        const double DoubleInfinity = double.PositiveInfinity;
        const double DoubleNegativeInfinity = double.NegativeInfinity;
        const double DoubleNegativeZero = -0.0;

        const decimal DecimalZero = 0m;
        const decimal DecimalScaled = 12.750m;
        const decimal DecimalNegative = -0.0001m;
        const decimal DecimalMax = decimal.MaxValue;
        const decimal DecimalMin = decimal.MinValue;
        const decimal DecimalFraction = 0.1234567890123456789012345678m;

        const string Empty = "";
        const string Ascii = "constant";
        const string Accents = "Grüße, café";
        const string Symbols = "€ ∑ 日本語";
        const string Surrogates = "😀";
        const string EmbeddedNul = "a\0b";

        const char Letter = 'A';
        const char CharZero = '\0';
        const char CharEuro = '€';
        const char CharMax = '\uffff';

        const sbyte SByteMin = sbyte.MinValue;
        const sbyte SByteMax = sbyte.MaxValue;
        const byte ByteMax = byte.MaxValue;
        const short Int16Min = short.MinValue;
        const short Int16Max = short.MaxValue;
        const ushort UInt16Max = ushort.MaxValue;
        const int Int32Min = int.MinValue;
        const int Int32Max = int.MaxValue;
        const uint UInt32Max = uint.MaxValue;
        const long Int64Min = long.MinValue;
        const long Int64Max = long.MaxValue;
        const ulong UInt64Max = ulong.MaxValue;
        const int Zero = 0;
        const int Small = 0x7fff;
        const int Medium = 0x8000;
        const nint NativeInt = -1;
        const nuint NativeUInt = 1;

        const bool True = true;
        const bool False = false;

        const string? NullString = null;
        const object? NullObject = null;
        const int[]? NullArray = null;
        const Type? NullType = null;

        const DayOfWeek Day = DayOfWeek.Saturday;
        const Level LevelLow = Level.Low;
        const Mask MaskAll = Mask.All;
        const Offset OffsetBack = Offset.Back;

        object?[] values =
        {
            Single, SingleNegative, SingleMax, SingleEpsilon, SingleNaN, SingleInfinity,
            SingleNegativeZero, Double, DoubleMin, DoubleEpsilon, DoubleNaN, DoubleInfinity,
            DoubleNegativeInfinity, DoubleNegativeZero, DecimalZero, DecimalScaled,
            DecimalNegative, DecimalMax, DecimalMin, DecimalFraction, Empty, Ascii, Accents,
            Symbols, Surrogates, EmbeddedNul, Letter, CharZero, CharEuro, CharMax, SByteMin,
            SByteMax, ByteMax, Int16Min, Int16Max, UInt16Max, Int32Min, Int32Max, UInt32Max,
            Int64Min, Int64Max, UInt64Max, Zero, Small, Medium, NativeInt, NativeUInt, True,
            False, NullString, NullObject, NullArray, NullType, Day, LevelLow, MaskAll,
            OffsetBack,
        };

        int failures = values.Length == 57 ? 0 : 1;
        failures += float.IsNaN(SingleNaN) && double.IsNaN(DoubleNaN) ? 0 : 1;
        failures += float.IsNegative(SingleNegativeZero) && double.IsNegative(DoubleNegativeZero)
            ? 0
            : 1;
        failures += decimal.GetBits(DecimalScaled)[3] == 3 << 16 ? 0 : 1;
        return failures;
    }
}
