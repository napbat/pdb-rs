/* Declarations that the C translation unit shares with the C++ translation units. */
#ifndef PDB_RS_DEMO_C_H
#define PDB_RS_DEMO_C_H

#ifdef __cplusplus
extern "C" {
#endif

enum c_mode {
    C_MODE_IDLE = 0,
    C_MODE_FAST = 3,
    C_MODE_SAFE = 7
};

struct c_point {
    int x;
    int y;
};

union c_value {
    int as_int;
    float as_float;
    unsigned char bytes[4];
};

struct c_record {
    const char *label;
    struct c_point origin;
    union c_value value;
    enum c_mode mode;
    unsigned short flags : 4;
    unsigned short level : 12;
};

extern struct c_point g_c_origin;
extern union c_value g_c_value;
extern enum c_mode g_c_mode;
extern const char *g_c_name;

int c_point_distance2(const struct c_point *a, const struct c_point *b);
int c_describe(struct c_record *record);
int c_sum(int count, ...);

#ifdef __cplusplus
}
#endif

#endif
