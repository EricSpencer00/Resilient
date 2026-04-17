// RES-082: VM-compatible fib for the --vm microbench.
//
// Differences from fib.rs:
//   - No `requires n >= 0` — contract clauses aren't lowered to
//     bytecode yet (that's a later ticket).
//   - No `println(fib(25))` — builtin calls aren't a VM op yet.
//     Instead, the bare `fib(25);` expression statement leaks the
//     return value to the stack, which the top-level Op::Return
//     surfaces as the program's exit value (printed by the driver).
//
// Same `n` as fib.rs so the tree-walker vs VM comparison is
// apples-to-apples.
fn fib(int n) {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

fib(25);
