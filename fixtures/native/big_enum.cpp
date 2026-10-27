// An enum with 10,000 enumerators. Its field list is larger than one type record, so the
// compiler splits it with LF_INDEX continuation records.
#include "demo.h"

// Each level pastes one more digit to the prefix.
#define PDB_RS_E10(p) p##0, p##1, p##2, p##3, p##4, p##5, p##6, p##7, p##8, p##9,
#define PDB_RS_E100(p)                                                                          \
    PDB_RS_E10(p##0) PDB_RS_E10(p##1) PDB_RS_E10(p##2) PDB_RS_E10(p##3) PDB_RS_E10(p##4)            \
    PDB_RS_E10(p##5) PDB_RS_E10(p##6) PDB_RS_E10(p##7) PDB_RS_E10(p##8) PDB_RS_E10(p##9)
#define PDB_RS_E1000(p)                                                                         \
    PDB_RS_E100(p##0) PDB_RS_E100(p##1) PDB_RS_E100(p##2) PDB_RS_E100(p##3) PDB_RS_E100(p##4)       \
    PDB_RS_E100(p##5) PDB_RS_E100(p##6) PDB_RS_E100(p##7) PDB_RS_E100(p##8) PDB_RS_E100(p##9)
#define PDB_RS_E10000(p)                                                                        \
    PDB_RS_E1000(p##0) PDB_RS_E1000(p##1) PDB_RS_E1000(p##2) PDB_RS_E1000(p##3) PDB_RS_E1000(p##4)  \
    PDB_RS_E1000(p##5) PDB_RS_E1000(p##6) PDB_RS_E1000(p##7) PDB_RS_E1000(p##8) PDB_RS_E1000(p##9)

namespace demo::big {

enum class Huge : std::uint16_t { PDB_RS_E10000(Item_) Count };

static_assert(static_cast<int>(Huge::Count) == 10000, "the enum must have 10,000 enumerators");

Huge g_huge = Huge::Item_9999;

}  // namespace demo::big

namespace demo {

int run_big_enum() { return static_cast<int>(big::g_huge); }

}  // namespace demo
