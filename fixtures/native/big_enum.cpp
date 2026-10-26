// An enum with 10,000 enumerators. Its field list is larger than one type record, so the
// compiler splits it with LF_INDEX continuation records.
#include "demo.h"

// Each level pastes one more digit to the prefix.
#define PDBRS_E10(p) p##0, p##1, p##2, p##3, p##4, p##5, p##6, p##7, p##8, p##9,
#define PDBRS_E100(p)                                                                          \
    PDBRS_E10(p##0) PDBRS_E10(p##1) PDBRS_E10(p##2) PDBRS_E10(p##3) PDBRS_E10(p##4)            \
    PDBRS_E10(p##5) PDBRS_E10(p##6) PDBRS_E10(p##7) PDBRS_E10(p##8) PDBRS_E10(p##9)
#define PDBRS_E1000(p)                                                                         \
    PDBRS_E100(p##0) PDBRS_E100(p##1) PDBRS_E100(p##2) PDBRS_E100(p##3) PDBRS_E100(p##4)       \
    PDBRS_E100(p##5) PDBRS_E100(p##6) PDBRS_E100(p##7) PDBRS_E100(p##8) PDBRS_E100(p##9)
#define PDBRS_E10000(p)                                                                        \
    PDBRS_E1000(p##0) PDBRS_E1000(p##1) PDBRS_E1000(p##2) PDBRS_E1000(p##3) PDBRS_E1000(p##4)  \
    PDBRS_E1000(p##5) PDBRS_E1000(p##6) PDBRS_E1000(p##7) PDBRS_E1000(p##8) PDBRS_E1000(p##9)

namespace demo::big {

enum class Huge : std::uint16_t { PDBRS_E10000(Item_) Count };

static_assert(static_cast<int>(Huge::Count) == 10000, "the enum must have 10,000 enumerators");

Huge g_huge = Huge::Item_9999;

}  // namespace demo::big

namespace demo {

int run_big_enum() { return static_cast<int>(big::g_huge); }

}  // namespace demo
