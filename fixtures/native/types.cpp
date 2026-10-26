// Type records: enums, bitfields, unions, classes, inheritance, templates and pointers.
#include "demo.h"

#include <cstdio>
#include <utility>

namespace demo {

namespace {

// Names in an anonymous namespace get internal linkage.
int hidden_counter = 0;

struct HiddenHelper {
    int value;
    int twice() const { return value * 2; }
};

}  // namespace

typedef unsigned int Flags;
using Callback = int (*)(int, int);
constexpr int kAnswer = 42;
constexpr double kScale = 2.5;

namespace enums {

enum Color { Red, Green = 5, Blue };

enum class Signed8 : std::int8_t { Min = -128, MinusTwo = -2, MinusOne = -1, Zero = 0, Max = 127 };

enum class Wide64 : std::uint64_t {
    Small = 1,
    Large = 0xFEDCBA9876543210ull,
    Max = 0xFFFFFFFFFFFFFFFFull,
};

}  // namespace enums

struct PackedBits {
    unsigned int ready : 1;
    unsigned int mode : 3;
    unsigned int count : 12;
    signed int delta : 5;
    std::uint64_t wide : 40;
};

union Number {
    std::int32_t i;
    float f;
    unsigned char bytes[4];
};

struct Variant {
    int kind;
    union {
        int as_int;
        double as_double;
        struct {
            short low;
            short high;
        };
    };
};

namespace shapes {

class Circle final : public Shape {
public:
    explicit Circle(double radius) : Shape(1), radius_(radius) { ++instances; }
    ~Circle() override { ++hidden_counter; }
    double area() const override { return 3.25 * radius_ * radius_; }
    const char* name() const override { return "circle"; }

    static int instances;
    static constexpr int kSides = 0;

private:
    double radius_;
};

int Circle::instances = 0;

class Rect : public Shape {
public:
    // A nested class with its own nested enum.
    class Corner {
    public:
        enum Side { Left, Right };
        Corner(int x, int y, Side side) : x_(x), y_(y), side_(side) {}
        int weight() const { return x_ + y_ + static_cast<int>(side_); }

    private:
        int x_;
        int y_;
        Side side_;
    };

    Rect(double width, double height) : Shape(2), width_(width), height_(height) {}
    double area() const override { return width_ * height_; }
    const char* name() const override { return "rect"; }
    Corner corner() const { return Corner(1, 2, Corner::Right); }

private:
    double width_;
    double height_;
};

}  // namespace shapes

// A diamond with virtual base classes.
struct Node {
    virtual ~Node() = default;
    virtual int weight() const { return 1; }
    int node_value = 1;
};

struct LeftNode : virtual Node {
    int left = 2;
    int weight() const override { return 2; }
};

struct RightNode : virtual Node {
    int right = 3;
};

struct DiamondNode : LeftNode, RightNode {
    int bottom = 4;
    int weight() const override { return left + right + bottom + node_value; }
};

class Vec2 {
public:
    Vec2() : x(0), y(0) {}
    Vec2(float x_value, float y_value) : x(x_value), y(y_value) {}
    Vec2(const Vec2&) = default;
    Vec2(Vec2&& other) noexcept : x(other.x), y(other.y) { other.x = other.y = 0; }
    ~Vec2() { ++hidden_counter; }

    Vec2& operator=(const Vec2&) = default;
    Vec2 operator+(const Vec2& other) const { return Vec2(x + other.x, y + other.y); }
    Vec2 operator*(float factor) const { return Vec2(x * factor, y * factor); }
    Vec2& operator+=(const Vec2& other) {
        x += other.x;
        y += other.y;
        return *this;
    }
    bool operator==(const Vec2& other) const { return x == other.x && y == other.y; }
    float operator[](int index) const { return index == 0 ? x : y; }
    explicit operator bool() const { return x != 0 || y != 0; }
    operator double() const { return static_cast<double>(x) + y; }

    void scale(float factor) { x *= factor, y *= factor; }
    void scale(float x_factor, float y_factor) { x *= x_factor, y *= y_factor; }
    void scale(const Vec2& factors) { scale(factors.x, factors.y); }

    float x;
    float y;
};

template <typename T, int N>
struct FixedArray {
    T items[N];
    constexpr int size() const { return N; }
    T sum() const {
        T total{};
        for (const T& item : items) total += item;
        return total;
    }
};

template <>
struct FixedArray<bool, 8> {
    unsigned char bits;
    constexpr int size() const { return 8; }
    int sum() const {
        int total = 0;
        for (int bit = 0; bit < 8; ++bit) total += (bits >> bit) & 1;
        return total;
    }
};

template <typename T>
T max_of(T a, T b) {
    return a < b ? b : a;
}

template <typename... Ts>
auto sum_all(Ts... values) {
    return (values + ... + 0);
}

template <typename T>
struct TypeTag {
    static const char* name() { return "unknown"; }
};

template <>
struct TypeTag<int> {
    static const char* name() { return "int"; }
};

struct Point {
    int x;
    int y;
    int sum() const { return x + y; }
    int product() const { return x * y; }
};

int add_ints(int a, int b) { return a + b; }

int Point::*g_member = &Point::y;
int (Point::*g_method)() const = &Point::sum;
Callback g_callback = &add_ints;
int g_matrix[3][4][5];
double g_grid[2][3] = {{1.0, 2.0, 3.0}, {4.0, 5.0, 6.0}};
Flags g_flags = 0x5u;

long double g_long_double = 1.5L;
wchar_t g_wide = L'W';
char8_t g_char8 = u8'8';
char16_t g_char16 = u'X';
char32_t g_char32 = U'Y';
enums::Color g_color = enums::Blue;
enums::Signed8 g_signed8 = enums::Signed8::MinusTwo;
enums::Wide64 g_wide64 = enums::Wide64::Large;

const volatile unsigned long long g_cv_wide = 0x1234ull;

__declspec(noinline) int qualifiers(const int& cref, int&& rref, volatile int* vptr,
                                    int* __restrict rptr, const volatile unsigned long long* cv) {
    *rptr += cref + rref;
    *vptr = *rptr;
    return *vptr + static_cast<int>(*cv & 0xff);
}

__declspec(noinline) int bit_and_union_work() {
    PackedBits bits{};
    bits.ready = 1;
    bits.mode = 5;
    bits.count = 1000;
    bits.delta = -7;
    bits.wide = 0x12345678ull;
    Number number{};
    number.f = 1.0f;
    Variant variant{};
    variant.kind = 2;
    variant.low = 3;
    variant.high = 4;
    return static_cast<int>(bits.ready + bits.mode + bits.count) + bits.delta + number.bytes[3] +
           variant.low + variant.high + static_cast<int>(bits.wide & 0xf);
}

__declspec(noinline) int class_work() {
    shapes::Circle circle(2.0);
    shapes::Rect rect(3.0, 4.0);
    const shapes::Shape* all[] = {&circle, &rect};
    double total = 0;
    for (const shapes::Shape* shape : all) total += shape->area() + shape->id();
    DiamondNode diamond;
    Node& base = diamond;
    HiddenHelper helper{kAnswer};
    return static_cast<int>(total) + base.weight() + rect.corner().weight() + helper.twice() +
           shapes::Circle::kSides + shapes::Circle::instances;
}

__declspec(noinline) int operator_work() {
    Vec2 a(1, 2);
    Vec2 b(3, 4);
    Vec2 c = a + b * 2.0f;
    c += Vec2(1, 1);
    c.scale(2.0f);
    c.scale(1.0f, 0.5f);
    c.scale(Vec2(1, 1));
    Vec2 moved(std::move(c));
    double as_double = moved;
    return static_cast<int>(as_double + moved[0]) + (moved == a ? 1 : 0) + (moved ? 1 : 0);
}

__declspec(noinline) int template_work() {
    FixedArray<int, 4> ints{{1, 2, 3, 4}};
    FixedArray<double, 2> doubles{{0.5, 1.5}};
    FixedArray<bool, 8> flags{0x0f};
    return ints.sum() + static_cast<int>(doubles.sum()) + flags.sum() + ints.size() +
           max_of(3, 9) + static_cast<int>(max_of(2.5, 1.5)) + sum_all(1, 2, 3, 4) +
           TypeTag<int>::name()[0] + TypeTag<float>::name()[0];
}

__declspec(noinline) int pointer_work() {
    Point point{6, 7};
    int value = 5;
    int target = 1;
    volatile int sink = 0;
    g_matrix[2][3][4] = point.*g_member;
    return (point.*g_method)() + g_callback(2, 3) + g_matrix[2][3][4] +
           qualifiers(value, 8, &sink, &target, &g_cv_wide) + static_cast<int>(g_grid[1][2]);
}

int run_types() {
    int scalars = static_cast<int>(g_long_double) + static_cast<int>(g_wide) + g_char8 +
                  static_cast<int>(g_char16) + static_cast<int>(g_char32) + g_color +
                  static_cast<int>(g_signed8) + static_cast<int>(static_cast<std::uint64_t>(g_wide64) & 0xff) +
                  static_cast<int>(g_flags) + static_cast<int>(kScale);
    return scalars + bit_and_union_work() + class_work() + operator_work() + template_work() +
           pointer_work() + hidden_counter;
}

}  // namespace demo
