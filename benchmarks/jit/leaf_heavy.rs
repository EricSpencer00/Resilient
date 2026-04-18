// RES-175: leaf-fn-inliner microbenchmark. Calls a tiny leaf fn
// in a tight loop — exactly the shape the inliner targets.
// Without the inliner each call is an indirect-call shim; with
// it, the body splices in-line at the call site.
//
// Iteration count (10M) is chosen to amortize the ~4 ms
// per-run JIT compile overhead, so the delta between inlined
// and non-inlined runs is visible above noise.
//
// Expected result: sum_{i=0..10_000_000}(i+1)
//   = 10_000_001 * 10_000_000 / 2
//   = 50_000_005_000_000.

fn plus_one(int x) { return x + 1; }

fn loop_bump() {
    let acc = 0;
    let i = 0;
    while i < 10000000 {
        acc = acc + plus_one(i);
        i = i + 1;
    }
    return acc;
}

return loop_bump();
