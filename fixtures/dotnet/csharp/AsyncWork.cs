// Async methods, closures captured across await points and the async part of the stack mode.
using System.Threading.Tasks;

namespace PdbrsDemo;

internal static class AsyncWork
{
    public static async Task<int> ComputeAsync(int seed)
    {
        int captured = seed * 2;
        await Task.Yield();
        int first = await DoubleAsync(captured);
        await Task.Delay(1);
        return first + captured;
    }

    private static async Task<int> DoubleAsync(int value)
    {
        await Task.Yield();
        return value * 2;
    }

    // The stack mode enumerates an iterator in Program.cs before the first await.
    public static async Task<int> TraceAsync(int depth)
    {
        int sum = 0;
        foreach (int value in Sequences.Traced(depth))
        {
            sum += value;
        }

        await Task.Yield();
        return sum;
    }
}
