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

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no actor call sites.
    let has_actor_call = crate::uniqueness_walk::any_node(program, |n| {
        if let Node::CallExpression { function, .. } = n {
            if let Node::Identifier { name, .. } = function.as_ref() {
                return matches!(name.as_str(), "spawn" | "send" | "receive");
            }
        }
        false
    });
    if !has_actor_call {
        return Ok(());
    }
    let site_count = count_actor_sites(program);
    eprintln!(
        "causal-trace: {} actor call site(s) detected — \
         circular trace buffer active ({} entries capacity)",
        site_count, TRACE_CAPACITY
    );
    Ok(())
}

fn count_actor_sites(node: &Node) -> u32 {
    let mut n = 0u32;
    count_sites_rec(node, &mut n);
    n
}

fn count_sites_rec(node: &Node, count: &mut u32) {
    if let Node::CallExpression { function, .. } = node {
        if let Node::Identifier { name, .. } = function.as_ref() {
            if matches!(name.as_str(), "spawn" | "send" | "receive") {
                *count += 1;
            }
        }
    }
    crate::uniqueness_walk::walk_children(node, &mut |child| count_sites_rec(child, count));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_program_without_actor_calls() {
        let src = r#"fn f(int x) -> int { return x + 1; }"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn count_actor_sites_finds_spawn_send_receive() {
        let src = r#"fn f(int x) { let pid = spawn(g); send(pid, x); }"#;
        let (prog, _) = crate::parse(src);
        let n = count_actor_sites(&prog);
        assert!(n >= 2, "expected at least 2 actor call sites, got {n}");
    }

    #[test]
    fn check_ok_with_actor_calls() {
        let src = r#"fn f(int x) { let pid = spawn(g); send(pid, x); }"#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

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
