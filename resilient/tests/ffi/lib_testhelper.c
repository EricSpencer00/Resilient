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

/* RES-FFI-V3: arity 4–8 sum helpers, exercised by ffi_trampolines.rs
 * tests via libloading. Each variadic-looking signature is a fixed
 * arity and simply sums its inputs — handy for confirming the
 * trampoline handed each argument to the right register slot.
 */
int64_t rt_sum_4(int64_t a, int64_t b, int64_t c, int64_t d) {
    return a + b + c + d;
}

int64_t rt_sum_5(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e) {
    return a + b + c + d + e;
}

int64_t rt_sum_6(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e, int64_t f) {
    return a + b + c + d + e + f;
}

int64_t rt_sum_7(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e, int64_t f, int64_t g) {
    return a + b + c + d + e + f + g;
}

int64_t rt_sum_8(int64_t a, int64_t b, int64_t c, int64_t d, int64_t e, int64_t f, int64_t g, int64_t h) {
    return a + b + c + d + e + f + g + h;
}
