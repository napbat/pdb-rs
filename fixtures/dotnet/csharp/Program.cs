// The entry point of the C# demo. Without arguments, it prints a checksum and exits with code 0.
// With the argument "stack", it prints a stack trace with file names and line numbers.
using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace PdbrsDemo;

public static class Program
{
    public static async Task<int> Main(string[] args)
    {
        if (args.Length > 0 && args[0] == "stack")
        {
            await AsyncWork.TraceAsync(2);
            return 0;
        }

        const int Seed = 7;
        const string Label = "csharp";

        int offset = 3;
        Func<int, int> addOffset = value => value + offset;
        offset = 4;

        int LocalSquare(int value) => value * value;
        static int StaticLocalCube(int value) => value * value * value;

        (int Count, string Name) pair = (Count: Seed, Name: Label);

        dynamic dynamicValue = 5;
        int fromDynamic = dynamicValue * 2;

        int iteratorSum = 0;
        foreach (int value in Sequences.Evens(5))
        {
            iteratorSum += value;
        }

        int asyncResult = await AsyncWork.ComputeAsync(Seed);
        int shapeTotal = Shapes.Describe();
        int hidden = HiddenRegion.Compute(Seed);
        int locals = LocalInfo.Compute(Seed);
        int counted = new StepCounter(2).Next() + MappedRegion.Compute(0);

        int total = addOffset(1) + LocalSquare(pair.Count) + StaticLocalCube(2) + fromDynamic
            + iteratorSum + asyncResult + shapeTotal + hidden + locals + counted + pair.Name.Length;
        Console.WriteLine($"csharp demo checksum {total}");
        return 0;
    }
}

internal static class Sequences
{
    public static IEnumerable<int> Evens(int count)
    {
        for (int index = 0; index < count; index++)
        {
            yield return index * 2;
        }
    }

    // The stack mode calls this iterator from an async method in AsyncWork.cs.
    public static IEnumerable<int> Traced(int depth)
    {
        for (int index = 0; index < depth; index++)
        {
            if (index == depth - 1)
            {
                yield return Shapes.PrintTrace();
            }
            else
            {
                yield return index;
            }
        }
    }
}

internal static class HiddenRegion
{
    public static int Compute(int value)
    {
        int visible = value + 1;
#line hidden
        int hidden = visible * 3;
#line default
        return hidden - 1;
    }
}
