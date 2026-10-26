// The entry point of the native demo. It calls each translation unit and prints a checksum.
#include "demo.h"

#include <cstdio>

int main() {
    long long total = demo::run_types();
    total += demo::run_control();
    total += demo::run_big_enum();

    c_point a{1, 2};
    c_point b{4, 6};
    c_record record{"record", {g_c_origin.x, g_c_origin.y}, g_c_value, g_c_mode, 0, 0};
    total += c_point_distance2(&a, &b);
    total += c_describe(&record);
    total += c_sum(3, 10, 20, 30);
    total += g_c_name[0];

    std::printf("native demo checksum %lld\n", total);
    return 0;
}
