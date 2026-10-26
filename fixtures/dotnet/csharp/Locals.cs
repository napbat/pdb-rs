// Local constants, nested scopes, dynamic locals and tuple element names in a method without a
// state machine, so that the locals stay in local slots. It also declares an abstract method and
// an extern method. These methods have no sequence points. The alias and static imports give
// more import kinds.
using System;
using System.Runtime.InteropServices;
using static System.Math;
using Week = System.DayOfWeek;

namespace PdbrsDemo;

internal static class LocalInfo
{
    public static int Compute(int seed)
    {
        const int Limit = 3;
        const long Wide = 1L << 40;
        const double Ratio = 0.5;
        const decimal Price = 12.75m;
        const char Letter = 'p';
        const bool Enabled = true;
        const string Text = "locals";
        const string? NoText = null;
        const object? NoObject = null;
        const Week Day = Week.Friday;

        (int Left, int Right) pair = (Left: seed, Right: Limit);
        dynamic dynamicSeed = seed;
        dynamic[] dynamicArray = { 1, "two" };
        (dynamic Value, string Name) mixed = (Value: dynamicSeed, Name: Text);

        int total = 0;
        for (int outer = 0; outer < Limit; outer++)
        {
            const int Step = 2;
            int doubled = outer * Step;
            if (Enabled)
            {
                const string Inner = "inner";
                int shifted = doubled + Inner.Length;
                {
                    (string Label, int Size) nested = (Label: Inner, Size: shifted);
                    total += nested.Size;
                }
            }
        }

        int fromDynamic = dynamicSeed + dynamicArray.Length;
        string? text = NoText;
        object? value = NoObject;
        int extra = text is null && value is null ? 1 : 0;
        return Max(total, 0) + pair.Left + pair.Right + fromDynamic + mixed.Name.Length + extra
            + (int)(Wide >> 40) + (int)(Ratio * 2) + (int)Price + Letter + (int)Day;
    }
}

internal abstract class Counter
{
    public abstract int Next();
}

internal sealed class StepCounter(int step) : Counter
{
    private int value;

    public override int Next()
    {
        value += step;
        return value;
    }
}

internal static class NativeMethods
{
    // The demo never calls this method. It gives a method without a body.
    [DllImport("kernel32.dll")]
    internal static extern uint GetCurrentProcessId();
}
