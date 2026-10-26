// A mixed C++/CLI program for /clr. It has managed types, managed and native functions,
// managed locals and constants, so the PDB has managed symbols, IL lines and token maps.
#using <System.dll>

using namespace System;
using namespace System::Collections::Generic;

#pragma unmanaged
// A native function in the same translation unit.
static int native_mix(int value) {
    int total = 0;
    for (int index = 0; index < value; ++index) {
        total += index * 3;
    }
    return total;
}
#pragma managed

namespace Demo {

public enum class Color { Red = 1, Green = 2, Blue = 4 };

public value struct Point {
    int X;
    int Y;
    int Length2() { return X * X + Y * Y; }
};

public delegate int Transform(int value);

public interface class IShape {
    double Area();
};

public ref class Square : IShape {
public:
    literal int Sides = 4;
    static initonly String^ Label = "square";

    Square(double side) : side_(side) {}

    property double Side {
        double get() { return side_; }
        void set(double value) { side_ = value; }
    }

    virtual double Area() { return side_ * side_; }

private:
    double side_;
};

generic <typename T>
public ref class Box {
public:
    Box(T value) : value_(value) {}
    T Get() { return value_; }

private:
    T value_;
};

int Twice(int value) { return value * 2; }

int Run() {
    const int bias = 7;
    Square^ square = gcnew Square(3.0);
    square->Side = 2.5;
    Point point;
    point.X = 3;
    point.Y = 4;
    Transform^ transform = gcnew Transform(&Twice);
    List<int>^ values = gcnew List<int>();
    for (int index = 0; index < 5; ++index) {
        values->Add(transform(index));
    }
    Box<String^>^ box = gcnew Box<String^>(Square::Label);
    array<int>^ numbers = gcnew array<int>{1, 2, 3};
    int total = bias + point.Length2() + static_cast<int>(square->Area()) + Square::Sides;
    for each (int value in values) {
        total += value;
    }
    for each (int number in numbers) {
        total += number;
    }
    try {
        if (box->Get()->Length > 100) {
            throw gcnew InvalidOperationException("long label");
        }
    } catch (InvalidOperationException^ error) {
        total -= error->Message->Length;
    }
    total += static_cast<int>(Color::Blue) + native_mix(4);
    return total;
}

}  // namespace Demo

int main() {
    Console::WriteLine("clr demo checksum {0}", Demo::Run());
    return 0;
}
