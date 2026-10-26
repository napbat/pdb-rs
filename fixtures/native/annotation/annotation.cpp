// A small program that calls the __annotation intrinsic. The compiler writes an S_ANNOTATION
// record into the procedure of each call, and the linker adds an S_ANNOTATIONREF record to the
// global symbols for each one.

#include <intrin.h>

namespace demo_annotation {

int checked_add(int a, int b) {
    __annotation(L"Demo", L"Checked", L"add");
    return a + b;
}

int scaled(int value, int factor) {
    __annotation(L"Demo", L"Scale");
    __annotation(L"Demo", L"Factor", L"one", L"more");
    return value * factor;
}

}  // namespace demo_annotation

int main(int argc, char**) {
    return demo_annotation::scaled(demo_annotation::checked_add(argc, 2), 3) & 1;
}
