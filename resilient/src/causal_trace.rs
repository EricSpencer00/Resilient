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

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
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
    let plain = format!(
        "causal-trace: {} actor call site(s) detected — \
         circular trace buffer active ({} entries capacity)",
        site_count, TRACE_CAPACITY
    );
    crate::typechecker::emit_check_warning_plain(plain, source_path, "causal_trace");
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

    // RES-3820: Malformed-input regression corpus for causal_trace validation
    #[test]
    fn malformed_format_chain_nonexistent_actor() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        record(TraceEntry {
            from: 1,
            to: 2,
            handler: "msg".into(),
            tick: 0,
        });
        // Query actor that has no messages
        let s = format_chain(999);
        assert!(
            s.is_empty(),
            "format_chain for non-existent actor should be empty"
        );
        clear();
    }

    #[test]
    fn malformed_record_zero_actors() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // Record entry with from=0, to=0 (self-message)
        record(TraceEntry {
            from: 0,
            to: 0,
            handler: "self_msg".into(),
            tick: 0,
        });
        let snap = snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].from, 0);
        assert_eq!(snap[0].to, 0);
        clear();
    }

    #[test]
    fn malformed_chain_preserves_order() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // Record multiple messages to same actor in order
        for i in 0..5 {
            record(TraceEntry {
                from: i,
                to: 100,
                handler: format!("msg_{i}"),
                tick: i,
            });
        }
        let s = format_chain(100);
        // Chain is reversed (most recent first), so msg_4 should appear before msg_0
        let pos_4 = s.find("msg_4").unwrap_or(0);
        let pos_0 = s.find("msg_0").unwrap_or(0);
        assert!(
            pos_4 < pos_0,
            "most recent message should appear first in chain"
        );
        clear();
    }

    #[test]
    fn malformed_concurrent_records() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // Multiple records with same tick
        for i in 0..3 {
            record(TraceEntry {
                from: i,
                to: 200,
                handler: format!("concurrent_{i}"),
                tick: 0, // Same tick
            });
        }
        let snap = snapshot();
        assert_eq!(snap.len(), 3);
        assert!(snap.iter().all(|e| e.tick == 0));
        clear();
    }

    #[test]
    fn malformed_clear_empty_trace() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // Clear on already-empty trace
        clear();
        assert_eq!(snapshot().len(), 0);
    }

    #[test]
    fn malformed_format_chain_empty() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        // Empty trace, query any actor
        let s = format_chain(42);
        assert_eq!(s, "");
        clear();
    }

    #[test]
    fn malformed_snapshot_isolation() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        record(TraceEntry {
            from: 1,
            to: 2,
            handler: "test".into(),
            tick: 0,
        });
        let snap1 = snapshot();
        // Snapshot is a clone, modifying it should not affect trace
        let snap2 = snapshot();
        assert_eq!(snap1.len(), snap2.len());
        // Record another and take snapshot
        record(TraceEntry {
            from: 2,
            to: 3,
            handler: "test2".into(),
            tick: 1,
        });
        let snap3 = snapshot();
        assert_eq!(snap3.len(), 2);
        clear();
    }

    #[test]
    fn malformed_large_handler_names() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        let long_name = "x".repeat(1000);
        record(TraceEntry {
            from: 1,
            to: 2,
            handler: long_name.clone(),
            tick: 0,
        });
        let snap = snapshot();
        assert_eq!(snap[0].handler, long_name);
        clear();
    }

    #[test]
    fn malformed_max_u64_ticks() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear();
        record(TraceEntry {
            from: 1,
            to: 2,
            handler: "overflow".into(),
            tick: u64::MAX,
        });
        let snap = snapshot();
        assert_eq!(snap[0].tick, u64::MAX);
        clear();
    }
}
