// RES-106: JIT-compatible fib for the --jit microbench.
//
// Same body as fib_vm.rs (no `requires`, no `println`), but the
// top-level statement is `return fib(25);` rather than a bare
// expression statement. The JIT requires a top-level return to
// terminate `__resilient_main__` — RES-105 doesn't yet treat
// expression statements as implicit returns.
//
// fib(25) = 75025; ~242,785 recursive calls — same workload
// fib_vm.rs uses, so the JIT row in RESULTS.md is comparable.
fn fib(int n) {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

return fib(25);
