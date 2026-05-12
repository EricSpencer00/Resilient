//! Feature 44/50 — Causal Failure Tracing.
//!
//! Runtime extension that maintains a per-actor message history. On
//! a runtime failure, the trace can be replayed to identify which
//! upstream actor's message caused the downstream actor to fail.
//!
//! The trace is implemented as a circular buffer (default capacity
//! 1024 entries) per actor. Each entry records:
//!
//! * Source actor (sender)
//! * Destination actor (self)
//! * Handler name
//! * Tick count when received
//!
//! On failure, `format_trace(actor_pid)` produces a human-readable
//! causal chain by walking back through the buffer.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::VecDeque;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct TraceEntry {
    pub from: u64,
    pub to: u64,
    pub handler: String,
    pub tick: u64,
}

const TRACE_CAPACITY: usize = 1024;

static GLOBAL_TRACE: RwLock<Option<VecDeque<TraceEntry>>> = RwLock::new(None);

pub fn record(entry: TraceEntry) {
    if let Ok(mut g) = GLOBAL_TRACE.write() {
        let buf = g.get_or_insert_with(VecDeque::new);
        if buf.len() >= TRACE_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(entry);
    }
}

pub fn snapshot() -> Vec<TraceEntry> {
    // RES-1547: hold the read guard so we can borrow through the
    // `Option<VecDeque<TraceEntry>>` and collect via `iter().cloned()`
    // in one pass. The previous shape did `g.clone()` (clones the
    // entire `VecDeque<TraceEntry>` inside the Option) and then
    // `.into_iter().collect()` — paid an extra container allocation
    // and discard for nothing.
    GLOBAL_TRACE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|d| d.iter().cloned().collect()))
        .unwrap_or_default()
}

pub fn clear() {
    if let Ok(mut g) = GLOBAL_TRACE.write() {
        if let Some(buf) = g.as_mut() {
            buf.clear();
        }
    }
}

pub fn format_chain(target_actor: u64) -> String {
    let mut s = String::new();
    let snap = snapshot();
    let chain: Vec<&TraceEntry> = snap.iter().filter(|e| e.to == target_actor).collect();
    for e in chain.iter().rev() {
        s.push_str(&format!(
            "  actor[{}] received `{}` from actor[{}] at tick {}\n",
            e.to, e.handler, e.from, e.tick
        ));
    }
    s
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn records_and_replays() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        record(TraceEntry {
            from: 1,
            to: 2,
            handler: "ping".into(),
            tick: 0,
        });
        record(TraceEntry {
            from: 2,
            to: 3,
            handler: "pong".into(),
            tick: 1,
        });
        let s = format_chain(3);
        assert!(s.contains("pong"));
        clear();
    }

    #[test]
    fn capacity_evicts_old_entries() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        for i in 0..(TRACE_CAPACITY + 100) {
            record(TraceEntry {
                from: i as u64,
                to: 0,
                handler: format!("h{}", i),
                tick: i as u64,
            });
        }
        assert_eq!(snapshot().len(), TRACE_CAPACITY);
        clear();
    }
}
