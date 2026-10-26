// Records and pattern matching.
namespace PdbrsDemo;

internal abstract record Shape(string Name);

internal sealed record Circle(double Radius) : Shape("circle");

internal sealed record Rect(double Width, double Height) : Shape("rect");

internal sealed record Empty() : Shape("empty");

internal static class Shapes
{
    public static int Describe()
    {
        Shape[] shapes = { new Circle(2.0), new Rect(3.0, 4.0), new Empty() };
        double total = 0;
        foreach (Shape shape in shapes)
        {
            total += Area(shape);
        }

        Rect copy = new Rect(1.0, 1.0) with { Height = 5.0 };
        return (int)total + (int)Area(copy) + Classify(17).Length;
    }

    private static double Area(Shape shape) => shape switch
    {
        Circle { Radius: var radius } => 3.25 * radius * radius,
        Rect(var width, var height) when width == height => width * width,
        Rect(var width, var height) => width * height,
        _ => 0.0,
    };

    private static string Classify(int value) => value switch
    {
        < 0 => "negative",
        0 => "zero",
        > 0 and < 10 => "small",
        _ => "large",
    };
}
