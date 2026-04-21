// RES-138: live_retries() inside a live block reports the current
// retry count — 0 on the first attempt, 1 on the first retry, etc.
//
// This example forces two failures and succeeds on the third
// attempt, printing `retry 0`, `retry 1`, `retry 2` before the
// success line. `static let` persists across retries; the live
// block's env snapshot resets only the regular bindings.

static let fails_left = 2;

fn maybe_fail() {
    if fails_left > 0 {
        fails_left = fails_left - 1;
        assert(false, "forced fail");
    }
    return 42;
}

fn main(int _d) {
    live {
        println("retry " + live_retries());
        let r = maybe_fail();
        println("succeeded with " + r);
    }
}

main(0);
