/* C types that clang-cl writes as primitive type indices that MSVC does not write:
 * __int128 (T_OCT, T_UOCT), _Float16 (T_REAL16), _Complex (T_CPLX32, T_CPLX64, Complex16),
 * and with -mlong-double-80 a 16-byte long double (T_REAL128, T_CPLX128).
 * kinds.f90 adds the Fortran kinds. The program does not convert _Float16 values, so it needs
 * no compiler runtime library. */
#include <stdio.h>
#include <string.h>

int touch_kinds(int seed);

__int128 g_i128 = 3;
unsigned __int128 g_u128 = 4;
_Float16 g_half;
long double g_long_double = 2.5L;
_Complex float g_complex_float = 1.0f;
_Complex double g_complex_double = 2.0;
_Complex long double g_complex_long_double = 3.0L;
_Complex _Float16 g_complex_half;

struct Mixed {
    __int128 wide;
    _Float16 half;
    long double extended;
    _Complex float complex_value;
};

struct Mixed g_mixed;

int use(struct Mixed *mixed, _Float16 *half, long double *extended) {
    int value;
    memcpy(&value, mixed, sizeof value);
    return value + (int)sizeof(*half) + (int)sizeof(*extended);
}

int main(void) {
    _Float16 half;
    long double extended = g_long_double;
    _Complex double complex_value = g_complex_double;
    __int128 wide = 0;
    memcpy(&half, &g_half, sizeof half);
    memcpy(&g_mixed.half, &half, sizeof half);
    g_mixed.extended = extended;
    g_mixed.wide = wide;
    printf("%d %d\n", use(&g_mixed, &half, &extended) + (int)extended + (int)__real__ complex_value,
           touch_kinds(3));
    return 0;
}
