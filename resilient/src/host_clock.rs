//! RES-3879: host clock abstraction.
//!
//! Native builds read the real monotonic (`Instant`) and wall
//! (`SystemTime`) clocks. On `wasm32-unknown-unknown` — the web
//! playground target — those APIs are unsupported and **panic** at
//! runtime (`Instant::now()` / `SystemTime::now()` trap with "time
//! not implemented on this platform"). A trap aborts the whole
//! playground run, which is strictly worse than the graceful `Err`
//! the fs / net builtins produce there. So on wasm this module
//! substitutes non-panicking software clocks: a monotonic counter and
//! a fixed-base wall clock. Time-using examples then run with
//! synthetic-but-monotonic values instead of crashing — the same
//! "demonstrate the language, not the host toolchain" posture as the
//! RES-3877 in-memory VFS.

use std::time::Duration;

/// Nanoseconds since an unspecified, process-lifetime monotonic
/// epoch. Only meaningful as a delta between two samples — callers
/// subtract to get an elapsed duration.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn monotonic_nanos() -> u128 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    Instant::now().duration_since(*epoch).as_nanos()
}

/// wasm: no host timer, so advance a process-global monotonic counter
/// by a fixed step per sample. Preserves the "monotonic, deltas-only"
/// contract (`clock_now`/`clock_ms`/`clock_elapsed`) without trapping.
#[cfg(target_arch = "wasm32")]
pub(crate) fn monotonic_nanos() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    /// One simulated millisecond of forward progress per sample, so
    /// two `clock_now()` calls straddling work observe a positive
    /// delta.
    const STEP_NS: u64 = 1_000_000;
    static COUNTER_NS: AtomicU64 = AtomicU64::new(0);
    COUNTER_NS.fetch_add(STEP_NS, Ordering::Relaxed) as u128
}

/// Duration since the Unix epoch (1970-01-01 UTC). Saturates to zero
/// for clocks set before the epoch, matching the `unix_time_*`
/// builtins' documented behavior.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn wall_clock_since_epoch() -> Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
}

/// wasm: anchor at a fixed, recent base epoch and advance with the
/// monotonic counter so timestamps are plausible, distinct, and
/// non-decreasing across calls without reading a (panicking) host
/// wall clock.
#[cfg(target_arch = "wasm32")]
pub(crate) fn wall_clock_since_epoch() -> Duration {
    /// 2026-01-01T00:00:00Z, chosen as a recent, legible anchor.
    const BASE_UNIX_SECS: u64 = 1_767_225_600;
    Duration::from_secs(BASE_UNIX_SECS) + Duration::from_nanos(monotonic_nanos() as u64)
}

/// Pause the current retry for `ms` milliseconds. Native: a real
/// `std::thread::sleep`. wasm: a no-op — `thread::sleep` panics on
/// `wasm32-unknown-unknown`, and a real sleep would block the
/// browser's single-threaded event loop anyway.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn sleep_ms(_ms: u64) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_is_non_decreasing() {
        let a = monotonic_nanos();
        let b = monotonic_nanos();
        assert!(b >= a, "monotonic clock went backwards: {a} -> {b}");
    }

    #[test]
    fn wall_clock_is_after_2020() {
        // 2020-01-01T00:00:00Z in seconds.
        let after_2020 = Duration::from_secs(1_577_836_800);
        assert!(
            wall_clock_since_epoch() >= after_2020,
            "wall clock predates 2020"
        );
    }

    #[test]
    fn sleep_ms_returns_promptly() {
        // Native sleeps a real 0ms; wasm is a no-op. Either way the
        // call must return without panicking.
        sleep_ms(0);
    }
}
