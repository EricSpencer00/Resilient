#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

int64_t rt_add(int64_t a, int64_t b) { return a + b; }
double  rt_mul(double a, double b)   { return a * b; }
bool    rt_is_even(int64_t n)        { return (n % 2) == 0; }

/* RES-317: C struct bridging — small structs ≤ 8 bytes by value.
 *
 * Resilient's Phase 1 trampoline lowers any `@repr(C)` struct that
 * fits in 8 bytes to a single u64 (INTEGER class on SystemV / Win-x64
 * / AArch64). The C side here defines structs with that exact layout
 * so the round-trip exercises real C code, not just a Rust transmute.
 */
typedef struct { int64_t v; } OneInt;

/* (Int) -> OneInt — factory shape. */
OneInt rt_make_one_int(int64_t v) {
    OneInt s; s.v = v; return s;
}

/* (OneInt) -> OneInt — read-modify-write round trip. */
OneInt rt_double_one_int(OneInt s) {
    OneInt out; out.v = s.v * 2; return out;
}

/* (OneInt) -> Int — readback. */
int64_t rt_one_int_value(OneInt s) {
    return s.v;
}
