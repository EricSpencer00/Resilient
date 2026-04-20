#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

int64_t rt_add(int64_t a, int64_t b) { return a + b; }
double  rt_mul(double a, double b)   { return a * b; }
bool    rt_is_even(int64_t n)        { return (n % 2) == 0; }
