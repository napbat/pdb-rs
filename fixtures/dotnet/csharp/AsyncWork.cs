// Async methods and closures captured across await points.
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
}
