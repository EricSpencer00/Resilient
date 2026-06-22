//! Grand-Implementation Pass 2 — Subsystem A: Logical Tick Clock + Bounded
//! Event Journal.
//!
//! Mainstream languages have wall-clock time, monotonic time, and (in some)
//! atomic counters. None has a built-in *deterministic-replay event journal*
//! at the core-stdlib level. OpenTelemetry / Jaeger do it as agents; LTTng,
//! perf, dtrace are kernel-level; eBPF is OS-specific. None ship with the
//! language as a first-class primitive.
//!
//! Resilient adds:
//!
//!   * `tick_now() -> Int` — read the monotonic logical tick counter.
//!   * `tick_advance(n: Int) -> Int` — advance the counter by `n`, return new value.
//!     The counter is purely logical (no wall-clock dependency) so replay
//!     across hosts produces identical timelines.
//!   * `record_event(name: String, payload: Int) -> Int` — append
//!     `(tick, name, payload)` to a thread-local bounded journal. Returns
//!     the event id (sequence number). Auto-advances the tick by 1.
//!   * `replay_events() -> Array<String>` — snapshot the journal as
//!     `"<id> [<tick>] <name>=<payload>"` strings, in append order.
//!   * `clear_events() -> Int` — drain the journal, return prior count.
//!   * `event_count() -> Int` — current number of events.
//!
//! The journal is bounded (`MAX_EVENTS`) so embedded targets cannot OOM.
//! Once full, oldest entries are dropped (FIFO) — preserving the most
//! recent context for postmortem.
//!
//! Why this is unique: the language itself enables deterministic structural
//! replay. Every interesting decision in your program can be `record_event`'d,
//! and the resulting journal is a first-class value you can compare against
//! a known-good trace. No agent, no library, no setup — it is part of the
//! language.

use crate::Value;
use std::cell::RefCell;

type RResult<T> = Result<T, String>;

const MAX_EVENTS: usize = 4096;

#[derive(Clone)]
struct Event {
    id: u64,
    tick: i64,
    name: String,
    payload: i64,
}

thread_local! {
    static TICK: RefCell<i64> = const { RefCell::new(0) };
    static NEXT_ID: RefCell<u64> = const { RefCell::new(0) };
    static JOURNAL: RefCell<std::collections::VecDeque<Event>> =
        const { RefCell::new(std::collections::VecDeque::new()) };
}

pub(crate) fn builtin_tick_now(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!("tick_now: expected 0 args, got {}", args.len()));
    }
    Ok(Value::Int(TICK.with(|t| *t.borrow())))
}

pub(crate) fn builtin_tick_advance(args: &[Value]) -> RResult<Value> {
    let n = match args {
        [Value::Int(n)] => *n,
        [a] => {
            return Err(format!("tick_advance: expected Int, got {}", type_name(a)));
        }
        _ => return Err(format!("tick_advance: expected 1 arg, got {}", args.len())),
    };
    if n < 0 {
        return Err("tick_advance: argument must be non-negative".to_string());
    }
    let next = TICK.with(|t| {
        let mut t = t.borrow_mut();
        *t = t.saturating_add(n);
        *t
    });
    Ok(Value::Int(next))
}

pub(crate) fn builtin_record_event(args: &[Value]) -> RResult<Value> {
    let (name, payload) = match args {
        [Value::String(n), Value::Int(p)] => (n.clone(), *p),
        [a, b] => {
            return Err(format!(
                "record_event: expected (String, Int), got ({}, {})",
                type_name(a),
                type_name(b)
            ));
        }
        _ => {
            return Err(format!("record_event: expected 2 args, got {}", args.len()));
        }
    };
    let id = NEXT_ID.with(|n| {
        let mut n = n.borrow_mut();
        let id = *n;
        *n = n.saturating_add(1);
        id
    });
    let tick = TICK.with(|t| {
        let mut t = t.borrow_mut();
        *t = t.saturating_add(1);
        *t
    });
    JOURNAL.with(|j| {
        let mut j = j.borrow_mut();
        if j.len() >= MAX_EVENTS {
            j.pop_front();
        }
        j.push_back(Event {
            id,
            tick,
            name,
            payload,
        });
    });
    Ok(Value::Int(id as i64))
}

pub(crate) fn builtin_replay_events(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "replay_events: expected 0 args, got {}",
            args.len()
        ));
    }
    let lines: Vec<Value> = JOURNAL.with(|j| {
        j.borrow()
            .iter()
            .map(|e| Value::String(format!("{} [{}] {}={}", e.id, e.tick, e.name, e.payload)))
            .collect()
    });
    Ok(Value::Array(lines))
}

pub(crate) fn builtin_clear_events(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!("clear_events: expected 0 args, got {}", args.len()));
    }
    let prior = JOURNAL.with(|j| {
        let mut j = j.borrow_mut();
        let n = j.len() as i64;
        j.clear();
        n
    });
    Ok(Value::Int(prior))
}

pub(crate) fn builtin_event_count(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!("event_count: expected 0 args, got {}", args.len()));
    }
    Ok(Value::Int(JOURNAL.with(|j| j.borrow().len() as i64)))
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::String(_) => "String",
        Value::Bool(_) => "Bool",
        Value::Array(_) => "Array",
        _ => "<value>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clear() {
        builtin_clear_events(&[]).unwrap();
        // Reset tick to 0 by using tick_advance(0) then accessing TICK directly.
        // Since tick only goes forward, we just clear events for isolation.
    }

    fn tick_now() -> i64 {
        match builtin_tick_now(&[]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        }
    }

    fn event_count() -> i64 {
        match builtin_event_count(&[]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        }
    }

    #[test]
    fn tick_now_returns_int() {
        let t = tick_now();
        assert!(t >= 0, "tick must be non-negative");
    }

    #[test]
    fn tick_advance_increases_tick() {
        let before = tick_now();
        let after = match builtin_tick_advance(&[Value::Int(3)]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        };
        assert_eq!(after, before + 3, "tick_advance must increase tick by n");
    }

    #[test]
    fn tick_advance_wrong_arity_errors() {
        let result = builtin_tick_advance(&[]);
        assert!(result.is_err(), "wrong arity must return Err");
    }

    #[test]
    fn record_and_count_events() {
        clear();
        let before = event_count();
        builtin_record_event(&[Value::String("test_evt".into()), Value::Int(99)]).unwrap();
        assert_eq!(
            event_count(),
            before + 1,
            "record_event must increment event count"
        );
    }

    #[test]
    fn clear_events_returns_prior_count() {
        clear();
        builtin_record_event(&[Value::String("e1".into()), Value::Int(1)]).unwrap();
        builtin_record_event(&[Value::String("e2".into()), Value::Int(2)]).unwrap();
        let count_before = event_count();
        let cleared = match builtin_clear_events(&[]).unwrap() {
            Value::Int(n) => n,
            _ => panic!("expected Int"),
        };
        assert_eq!(
            cleared, count_before,
            "clear_events must return prior event count"
        );
        assert_eq!(event_count(), 0, "after clear, event count must be 0");
    }

    // RES-3819: Malformed-input regression corpus for event_journal validation
    #[test]
    fn malformed_tick_advance_negative() {
        // tick_advance with negative argument should error
        let result = builtin_tick_advance(&[Value::Int(-1)]);
        assert!(result.is_err(), "tick_advance with negative n must error");
    }

    #[test]
    fn malformed_tick_advance_overflow() {
        // tick_advance with i64::MAX should saturate (not panic)
        let result = builtin_tick_advance(&[Value::Int(i64::MAX)]);
        assert!(result.is_ok(), "saturating addition must not panic");
    }

    #[test]
    fn malformed_tick_advance_wrong_type() {
        // tick_advance with String instead of Int
        let result = builtin_tick_advance(&[Value::String("oops".into())]);
        assert!(result.is_err(), "tick_advance type mismatch must error");
    }

    #[test]
    fn malformed_record_event_wrong_arity() {
        // record_event with 1 arg instead of 2
        let result = builtin_record_event(&[Value::String("e1".into())]);
        assert!(result.is_err(), "wrong arity must error");
    }

    #[test]
    fn malformed_record_event_swapped_types() {
        // record_event with (Int, String) instead of (String, Int)
        let result = builtin_record_event(&[Value::Int(42), Value::String("name".into())]);
        assert!(result.is_err(), "type order mismatch must error");
    }

    #[test]
    fn malformed_record_event_all_strings() {
        // record_event with (String, String)
        let result = builtin_record_event(&[Value::String("n".into()), Value::String("p".into())]);
        assert!(result.is_err(), "payload must be Int");
    }

    #[test]
    fn journal_overflow_fifo() {
        clear();
        // Fill journal to MAX_EVENTS + 1; oldest should be dropped
        for i in 0..=(MAX_EVENTS as i64) {
            builtin_record_event(&[Value::String(format!("evt_{i}")), Value::Int(i)]).unwrap();
        }
        // Count should be capped at MAX_EVENTS
        assert_eq!(event_count(), MAX_EVENTS as i64);
    }

    #[test]
    fn replay_events_format() {
        clear();
        builtin_record_event(&[Value::String("login".into()), Value::Int(1)]).unwrap();
        let replay = match builtin_replay_events(&[]).unwrap() {
            Value::Array(lines) => lines,
            _ => panic!("replay_events must return Array"),
        };
        assert_eq!(replay.len(), 1);
        // Format: "<id> [<tick>] <name>=<payload>"
        if let Value::String(s) = &replay[0] {
            assert!(s.contains('['), "replay format must include tick");
            assert!(s.contains('='), "replay format must include payload");
        }
    }

    #[test]
    fn replay_events_empty_array() {
        clear();
        let replay = match builtin_replay_events(&[]).unwrap() {
            Value::Array(lines) => lines,
            _ => panic!("replay_events must return Array"),
        };
        assert_eq!(replay.len(), 0, "empty journal produces empty array");
    }

    #[test]
    fn tick_advances_on_record() {
        clear();
        let before = tick_now();
        builtin_record_event(&[Value::String("e".into()), Value::Int(0)]).unwrap();
        let after = tick_now();
        assert_eq!(after, before + 1, "record_event must auto-advance tick");
    }

    #[test]
    fn record_event_id_increments() {
        clear();
        let id1 = match builtin_record_event(&[Value::String("e1".into()), Value::Int(1)]).unwrap()
        {
            Value::Int(id) => id,
            _ => panic!("record_event must return Int"),
        };
        let id2 = match builtin_record_event(&[Value::String("e2".into()), Value::Int(2)]).unwrap()
        {
            Value::Int(id) => id,
            _ => panic!("record_event must return Int"),
        };
        assert_eq!(id2, id1 + 1, "event IDs must increment sequentially");
    }

    #[test]
    fn malformed_builtin_calls_preserve_state() {
        clear();
        builtin_record_event(&[Value::String("e1".into()), Value::Int(1)]).unwrap();
        let before = event_count();
        // Failed call
        let _ = builtin_record_event(&[Value::String("e2".into())]);
        // State unchanged
        assert_eq!(
            event_count(),
            before,
            "failed record_event must not alter state"
        );
    }
}
