// RES-141: process-wide live-block telemetry counters exposed
// to programs as `live_total_retries()` / `live_total_exhaustions()`.
//
// This example reads the counters around a `live` block that
// fails twice before succeeding on the third attempt. The
// counters advance by 2 retries and 0 exhaustions (the block
// succeeds, so nothing "gave up").

static let fails_left = 2;

fn maybe_fail() {
    if fails_left > 0 {
        fails_left = fails_left - 1;
        assert(false, "forced");
    }
    return 42;
}

fn main(int _d) {
    println("before: retries=" + live_total_retries());
    println("before: exhaustions=" + live_total_exhaustions());

    live {
        let r = maybe_fail();
    }

    println("after:  retries=" + live_total_retries());
    println("after:  exhaustions=" + live_total_exhaustions());
}

main(0);
