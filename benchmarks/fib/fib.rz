// Recursive Fibonacci — classic interpreter stress test.
// Exponential calls; tests fn dispatch, env handling, recursion.

fn fib(int n) requires n >= 0 {
    if n < 2 {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}

println(fib(25));
