// A small ARM64EC program. The linker writes S_HYBRIDRANGE records for the code ranges of the
// ARM64EC image.
#include <cstdio>

__declspec(noinline) int work(int value) {
    int total = 0;
    for (int index = 0; index < value; ++index) {
        total += index * value;
    }
    return total;
}

int (*volatile indirect)(int) = work;

int main(int argc, char**) {
    std::printf("%d\n", indirect(argc + 3));
    return 0;
}
