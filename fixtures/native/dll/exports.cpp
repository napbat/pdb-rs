// A small DLL with many export forms. The C++ items use __declspec(dllexport). `exports.def`
// exports the C items with an ordinal, NONAME, PRIVATE, DATA, CONSTANT and a forwarder, so
// the linker writes S_EXPORT records with each flag.

extern "C" {

int demo_dll_counter = 3;
extern const int demo_dll_limit = 40;

int demo_dll_add(int a, int b) { return a + b + demo_dll_counter; }

int demo_dll_by_ordinal(int value) { return value * 2; }

int demo_dll_hidden(int value) { return value - 1; }

int demo_dll_private(int value) { return value ^ demo_dll_limit; }

}  // extern "C"

namespace demo_dll {

class __declspec(dllexport) Counter {
public:
    int next();

private:
    int value_ = 0;
};

int Counter::next() { return ++value_; }

__declspec(dllexport) int scale(int value) { return value * demo_dll_limit; }

}  // namespace demo_dll
