// A method whose sequence points change documents (#23). The #line directive maps part of the
// body to the document Mapped.template, #line hidden gives a hidden sequence point and
// #line default returns to this file. Compute returns its argument, so the checksum does not
// change.
namespace PdbrsDemo;

internal static class MappedRegion
{
    public static int Compute(int value)
    {
        int result = value + 1;
#line 100 "Mapped.template"
        result *= 2;
        result /= 2;
#line hidden
        result += 1;
#line default
        return result - 2;
    }
}
