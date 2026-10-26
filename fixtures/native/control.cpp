// Symbol records: thread storage, static locals, exceptions, labels, jump tables, calling
// conventions, inline sites, lambdas, standard containers and heap allocation sites.
#include "demo.h"

#include <functional>
#include <map>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

#if PDBRS_X86_FAMILY
#include <xmmintrin.h>
#endif

#if PDBRS_MSVC
#include <excpt.h>
#endif

namespace demo {

thread_local int tls_counter = 7;
thread_local std::uint64_t tls_wide = 0x1122334455667788ull;

#if PDBRS_X86_FAMILY
__m128 g_vector;
#endif

namespace {

__declspec(noinline) int next_sequence() {
    static int sequence = 100;
    thread_local int per_thread = 0;
    ++per_thread;
    return ++sequence + per_thread;
}

}  // namespace

__forceinline int inline_square(int value) { return value * value; }

__forceinline int inline_cube(int value) { return inline_square(value) * value; }

__declspec(noinline) int dense_switch(int value) {
    switch (value) {
        case 0: return 11;
        case 1: return 23;
        case 2: return 37;
        case 3: return 41;
        case 4: return 59;
        case 5: return 61;
        case 6: return 73;
        case 7: return 89;
        case 8: return 97;
        case 9: return 101;
        case 10: return 113;
        case 11: return 127;
        case 12: return 131;
        case 13: return 149;
        case 14: return 151;
        case 15: return 167;
        default: return -1;
    }
}

__declspec(noinline) int with_goto(int limit) {
    int total = 0;
    int index = 0;
loop_top:
    if (index >= limit) goto loop_done;
    total += index;
    ++index;
    goto loop_top;
loop_done:
    return total;
}

class DemoError : public std::runtime_error {
public:
    DemoError(const std::string& what, int code) : std::runtime_error(what), code(code) {}
    int code;
};

__declspec(noinline) int throw_and_catch(int value) {
    try {
        if (value > 0) throw DemoError("demo error", value);
        return 0;
    } catch (const DemoError& error) {
        return error.code;
    } catch (...) {
        return -1;
    }
}

#if PDBRS_MSVC
__declspec(noinline) int structured_exception(volatile int* pointer) {
    __try {
        return *pointer + 1;
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        return -2;
    }
}
#endif

__declspec(noinline) int __stdcall std_add(int a, int b) { return a + b; }

__declspec(noinline) int __fastcall fast_add(int a, int b, int c) { return a + b + c; }

#if PDBRS_X86_FAMILY
__declspec(noinline) __m128 __vectorcall vector_add(__m128 a, __m128 b) { return _mm_add_ps(a, b); }
#endif

__declspec(noinline) int containers() {
    std::vector<int> values{3, 1, 4, 1, 5};
    std::map<std::string, int> counts;
    std::string text = "pdbrs";
    for (char c : text) counts[std::string(1, c)] += 1;
    int base = 10;
    auto add_base = [base](int value) { return value + base; };
    auto accumulate = [&values]() {
        int sum = 0;
        for (int value : values) sum += value;
        return sum;
    };
    std::function<int(int)> function = add_base;
    auto generic = [](auto a, auto b) { return a * b; };
    return function(accumulate()) + static_cast<int>(counts.size()) + generic(2, 3);
}

__declspec(noinline) int heap_sites() {
    Widget* single = new Widget{5, nullptr};
    int* many = new int[16]{};
    std::unique_ptr<Widget> owned(new Widget{6, single});
    many[3] = owned->value + owned->next->value;
    int result = many[3];
    delete[] many;
    delete single;
    return result;
}

__declspec(noinline) int calling_conventions() {
    int total = std_add(1, 2) + fast_add(3, 4, 5);
#if PDBRS_X86_FAMILY
    g_vector = vector_add(_mm_set1_ps(1.0f), _mm_set1_ps(2.0f));
    total += static_cast<int>(_mm_cvtss_f32(g_vector));
#endif
    return total;
}

int run_control() {
    volatile int input = 6;
    int total = next_sequence() + next_sequence() + inline_cube(input) + dense_switch(input) +
                with_goto(input) + throw_and_catch(input) + containers() + heap_sites() +
                calling_conventions() + tls_counter + static_cast<int>(tls_wide & 0xff);
#if PDBRS_MSVC
    total += structured_exception(&input);
#endif
    return total;
}

}  // namespace demo
