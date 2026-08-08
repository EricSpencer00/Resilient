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

/* RES-4225: array (buffer) parameters.
 *
 * These are the shapes Resilient's `Array<Int>` / `Array<Float>` extern
 * parameters lower to: a `const T*` paired with a caller-supplied count.
 * `rt_sum_i64` also exercises the arity-only INTEGER-class dispatch path
 * with a mix of pointer and integer arguments.
 */
int64_t rt_sum_i64(const int64_t *xs, int64_t n) {
    int64_t acc = 0;
    for (int64_t i = 0; i < n; i++) acc += xs[i];
    return acc;
}

double rt_sum_f64(const double *xs, int64_t n) {
    double acc = 0.0;
    for (int64_t i = 0; i < n; i++) acc += xs[i];
    return acc;
}

/* Reads one element — proves the pointer is a real contiguous buffer and
 * not just a non-null address that happens to survive the call. */
int64_t rt_nth_i64(const int64_t *xs, int64_t i) {
    return xs[i];
}

/* Two buffers plus a count: a dot product. Confirms that two distinct
 * array arguments get two distinct live buffers, not one aliased twice. */
int64_t rt_dot_i64(const int64_t *xs, const int64_t *ys, int64_t n) {
    int64_t acc = 0;
    for (int64_t i = 0; i < n; i++) acc += xs[i] * ys[i];
    return acc;
}

/* Pointer-returning, pointer-and-buffer-taking: exercises the
 * OpaquePtr return shape through the word-class path. Returns the
 * buffer address it was handed, so the caller can verify pass-through. */
const int64_t *rt_echo_buf(const int64_t *xs, int64_t n) {
    (void)n;
    return xs;
}

/* Arity-8, all INTEGER-class, mixing buffers and counts. The upper bound
 * of the word-class dispatch table. */
int64_t rt_sum_two_bufs_8(const int64_t *a, int64_t na,
                          const int64_t *b, int64_t nb,
                          int64_t w0, int64_t w1, int64_t w2, int64_t w3) {
    int64_t acc = w0 + w1 + w2 + w3;
    for (int64_t i = 0; i < na; i++) acc += a[i];
    for (int64_t i = 0; i < nb; i++) acc += b[i];
    return acc;
}
