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
