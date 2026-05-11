//! RES-1174: wall-clock unix time builtins.
//!
//! Three @io / impure builtins that return seconds / milliseconds /
//! nanoseconds since the Unix epoch (1970-01-01 UTC).
//!
//! Companion to RES-147's `clock_ms` (monotonic since process start).
//! These three read the system wall clock — required for log
//! timestamps, file mtime comparisons, TLS expiry checks, distributed
//! Lamport-clock fallbacks, etc.
//!
//! | Builtin | Returns | Wraps at |
//! |---|---|---|
//! | `unix_time_s()`  | seconds since epoch  | year 292 277 026 596 (never) |
//! | `unix_time_ms()` | milliseconds since epoch | year 292 278 994 (never) |
//! | `unix_time_ns()` | nanoseconds since epoch  | year 2262 |
//!
//! All three clamp to `i64::MAX` on overflow rather than panicking,
//! consistent with `clock_ms`. Behavior on systems with a clock set
//! before the Unix epoch is well-defined (returns 0 — same as
//! `SystemTime::duration_since(UNIX_EPOCH)` saturating).

use crate::{RResult, Value};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(std::time::Duration::ZERO)
}

fn clamp_to_i64(n: u128) -> i64 {
    if n > i64::MAX as u128 {
        i64::MAX
    } else {
        n as i64
    }
}

/// `unix_time_s() -> Int` — seconds since 1970-01-01 UTC. Impure.
pub(crate) fn builtin_unix_time_s(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "unix_time_s: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let secs = now_unix().as_secs();
    Ok(Value::Int(clamp_to_i64(secs as u128)))
}

/// `unix_time_ms() -> Int` — milliseconds since 1970-01-01 UTC. Impure.
pub(crate) fn builtin_unix_time_ms(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "unix_time_ms: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let ms = now_unix().as_millis();
    Ok(Value::Int(clamp_to_i64(ms)))
}

/// `unix_time_ns() -> Int` — nanoseconds since 1970-01-01 UTC. Impure.
/// Will saturate at `i64::MAX` after the year 2262.
pub(crate) fn builtin_unix_time_ns(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "unix_time_ns: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let ns = now_unix().as_nanos();
    Ok(Value::Int(clamp_to_i64(ns)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn unix_time_s_positive() {
        let t = as_int(builtin_unix_time_s(&[]).unwrap());
        // Test runs after 2020 → should be > 1.6 billion seconds.
        assert!(t > 1_500_000_000, "unix_time_s = {}", t);
        // Test runs before some far-future date → should be < 4 billion.
        assert!(t < 4_000_000_000, "unix_time_s = {}", t);
    }

    #[test]
    fn unix_time_ms_consistent_with_s() {
        // ms must be roughly 1000× larger than s (allowing for timing slack).
        let s = as_int(builtin_unix_time_s(&[]).unwrap());
        let ms = as_int(builtin_unix_time_ms(&[]).unwrap());
        let delta_ms = ms - s * 1000;
        assert!(
            (0..1500).contains(&delta_ms),
            "ms ({}) and s ({}) should be ~1000:1, diff was {}",
            ms,
            s,
            delta_ms
        );
    }

    #[test]
    fn unix_time_ns_consistent_with_ms() {
        // ns / 1_000_000 ≈ ms.
        let ms = as_int(builtin_unix_time_ms(&[]).unwrap());
        let ns = as_int(builtin_unix_time_ns(&[]).unwrap());
        let derived_ms = ns / 1_000_000;
        let delta = derived_ms - ms;
        // Allow up to 1.5 seconds of slack for measurement gap.
        assert!(
            delta.abs() < 1500,
            "ns/1M ({}) and ms ({}) should match closely, delta {}",
            derived_ms,
            ms,
            delta
        );
    }

    #[test]
    fn unix_time_s_monotonic_or_equal() {
        // Two consecutive calls — second is >= first.
        let a = as_int(builtin_unix_time_s(&[]).unwrap());
        let b = as_int(builtin_unix_time_s(&[]).unwrap());
        assert!(b >= a);
    }

    #[test]
    fn unix_time_rejects_arguments() {
        let err = builtin_unix_time_s(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 0"));
        let err = builtin_unix_time_ms(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 0"));
        let err = builtin_unix_time_ns(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected 0"));
    }

    #[test]
    fn clamp_helper_handles_overflow() {
        assert_eq!(clamp_to_i64(0), 0);
        assert_eq!(clamp_to_i64(100), 100);
        assert_eq!(clamp_to_i64(i64::MAX as u128), i64::MAX);
        assert_eq!(clamp_to_i64((i64::MAX as u128) + 1), i64::MAX);
        assert_eq!(clamp_to_i64(u128::MAX), i64::MAX);
    }
}
