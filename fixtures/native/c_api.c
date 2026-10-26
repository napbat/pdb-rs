/* A C translation unit with structs, a union, an enum, globals and a variadic function. */
#include "demo_c.h"

#include <stdarg.h>

struct c_point g_c_origin = {2, 3};
union c_value g_c_value = {41};
enum c_mode g_c_mode = C_MODE_FAST;
const char *g_c_name = "c-unit";

static int s_c_calls;

int c_point_distance2(const struct c_point *a, const struct c_point *b)
{
    int dx = b->x - a->x;
    int dy = b->y - a->y;
    ++s_c_calls;
    return dx * dx + dy * dy;
}

int c_describe(struct c_record *record)
{
    int score = record->origin.x + record->origin.y + (int)record->mode;
    record->flags = 5;
    record->level = (unsigned short)(score & 0xfff);
    record->value.as_int += score;
    ++s_c_calls;
    return score + (record->label != 0 ? record->label[0] : 0);
}

int c_sum(int count, ...)
{
    va_list arguments;
    int total = 0;
    int index;
    va_start(arguments, count);
    for (index = 0; index < count; ++index) {
        total += va_arg(arguments, int);
    }
    va_end(arguments);
    ++s_c_calls;
    return total + s_c_calls;
}
