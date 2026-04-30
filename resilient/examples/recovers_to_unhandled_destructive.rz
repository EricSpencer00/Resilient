// RES-222: unhandled fault on a destructive path — Z3 rejects.
//
// `poll_sensor` declares `fails Timeout` and promises the recovery
// invariant `reading > 100`. The precondition only constrains
// `reading >= 0`, so Z3 cannot rule out a satisfying assignment
// where `reading` is between 0 and 100 at the recovery point. With
// no handler to catch the fault (structured handlers land in a
// separate ticket), the invariant is a mandatory obligation and
// the compiler refuses the program with a `recovers_to invariant
// cannot be proven` diagnostic that names the `fails` set and
// carries Z3's counterexample.
//
// File extension is `.res` so the goldens-match harness (which
// only walks `*.rz`) does not try to execute this example. A
// dedicated smoke test invokes the binary and asserts the
// diagnostic.
fn poll_sensor(int reading) -> int
    requires reading >= 0
    fails Timeout
    recovers_to: reading > 100;
{
    return reading;
}

fn main() fails Timeout {
    let v = poll_sensor(42);
    println(v);
}
