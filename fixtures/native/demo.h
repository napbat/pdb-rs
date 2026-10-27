// Declarations that the C++ translation units share.
#pragma once

#include <cstdint>

#include "demo_c.h"

#if defined(_MSC_VER) && !defined(__clang__)
#define PDB_RS_MSVC 1
#else
#define PDB_RS_MSVC 0
#endif

#if defined(_M_X64) || defined(_M_IX86)
#define PDB_RS_X86_FAMILY 1
#else
#define PDB_RS_X86_FAMILY 0
#endif

namespace demo {

// Each function returns a checksum so that the optimizer keeps the work.
int run_types();
int run_control();
int run_big_enum();

namespace shapes {

class Shape {
public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
    virtual const char* name() const = 0;
    int id() const { return id_; }

protected:
    explicit Shape(int id) : id_(id) {}

private:
    int id_;
};

}  // namespace shapes

// A heap-allocated type for the allocation-site records.
struct Widget {
    int value;
    Widget* next;
};

}  // namespace demo
