// C++20 features that give symbol kinds of recent MSVC versions:
// - a coroutine: S_ASSOCIATION in the ramp function,
// - structured bindings: S_REGREL32_INDIR (x64) and S_BPREL32_INDIR (x86),
// - references to temporaries with /ZI: S_BPREL32_ENCTMP (x86),
// - function-local statics with /O2: S_STATICLOCAL (the linker keeps it only with /DEBUG:LAZY).
#include <coroutine>
#include <cstdio>
#include <map>
#include <string>
#include <utility>

struct Generator {
    struct promise_type {
        int value = 0;
        Generator get_return_object() {
            return Generator{std::coroutine_handle<promise_type>::from_promise(*this)};
        }
        std::suspend_always initial_suspend() noexcept { return {}; }
        std::suspend_always final_suspend() noexcept { return {}; }
        std::suspend_always yield_value(int next) {
            value = next;
            return {};
        }
        void return_void() {}
        void unhandled_exception() {}
    };
    std::coroutine_handle<promise_type> handle;
    ~Generator() {
        if (handle) {
            handle.destroy();
        }
    }
};

Generator counter(int limit) {
    int offset = limit * 2;
    for (int index = 0; index < limit; ++index) {
        co_yield index + offset;
    }
}

__declspec(noinline) int statics(int value) {
    static int calls = 0;
    static thread_local int thread_calls = 0;
    static const int table[4] = {1, 2, 3, 4};
    calls += value;
    thread_calls += 1;
    return calls + thread_calls + table[value & 3];
}

std::pair<int, double> make_pair_value(int value) { return {value, value * 0.5}; }

__declspec(noinline) int bindings(int value) {
    auto [first, second] = make_pair_value(value);
    const auto& [left, right] = std::pair<int, int>{value, value + 1};
    std::map<std::string, int> names{{"a", 1}, {"b", 2}};
    int sum = 0;
    for (const auto& [key, number] : names) {
        sum += number + static_cast<int>(key.size());
    }
    const std::string& text = std::string("temporary") + std::to_string(value);
    return first + static_cast<int>(second) + left + right + sum + static_cast<int>(text.size());
}

int main(int argc, char**) {
    Generator generator = counter(argc + 2);
    int total = 0;
    while (!generator.handle.done()) {
        generator.handle.resume();
        total += generator.handle.promise().value;
    }
    total += statics(argc) + bindings(argc);
    std::printf("%d\n", total);
    return 0;
}
