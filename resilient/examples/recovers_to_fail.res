// RES-392 demo — `recovers_to` that the verifier must reject at
// runtime. The function declares a recovery invariant (`result == 0`
// — the "safe default") but always returns a non-zero value, so the
// final state falsifies the clause on every invocation.
//
// The Resilient driver surfaces this as:
//
//   Runtime error: Contract violation in fn init_actuator:
//     recovers_to result == 0 failed — final-state counterexample:
//     result = 3
//
// File extension is `.res` so the goldens-match harness (which walks
// only `*.rz`) does not try to run this example as a passing program.
// A dedicated smoke test (`recovers_to_smoke`) invokes the binary
// against it and asserts the failure diagnostic.
fn init_actuator(int id) -> int
    requires id >= 0
    recovers_to: result == 0;
{
    // Intentionally returns a non-safe value — the recovery
    // invariant claims `result == 0` but the body returns 3.
    return 3;
}

fn main() {
    let mode = init_actuator(1);
    println(mode);
}
main();
