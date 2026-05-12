//! Ralph-Loop Uniqueness #24 — priority-inheritance discipline.
//!
//! In real-time priority-based scheduling, a low-priority task holding
//! a lock can starve high-priority tasks that need the lock. RTOSes
//! (FreeRTOS, μC/OS) implement priority inheritance at the runtime;
//! POSIX has `PTHREAD_PRIO_INHERIT` you opt into. No mainstream
//! language *requires* a lock acquired by a `low_pri_*`/`bg_*` function
//! to be marked priority-inheriting.
//!
//! Resilient enforces by name: any function whose name starts with
//! `low_pri_`, `bg_`, or `idle_` and which calls `lock(...)` /
//! `acquire(...)` must wrap that call in `with_priority_inherit(...)`
//! or use `pi_lock(...)` / `pi_acquire(...)`. Otherwise we warn that
//! the lock will cause priority inversion.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

const LOW_PRI_PREFIXES: &[&str] = &["low_pri_", "bg_", "idle_"];
const PI_VARIANTS: &[&str] = &["pi_lock", "pi_acquire", "with_priority_inherit"];
const RAW_LOCKS: &[&str] = &["lock", "acquire", "mutex_lock"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1232: fast-reject. The per-function early-return inside
    // `for_each_function` already short-circuits the body walk for
    // any function whose name doesn't match `LOW_PRI_PREFIXES`, but
    // `for_each_function` itself is still entered for every program
    // and iterates every top-level statement. Programs with zero
    // `low_pri_*`/`bg_*`/`idle_*` functions (the overwhelming
    // majority of `cargo test` inputs and the entire `examples/`
    // tree) pay the iteration + closure-call cost for no work.
    // Hoist the suffix check above `for_each_function` so the whole
    // pass returns `Ok(())` immediately when no candidate exists.
    // Mirrors RES-1211 (isr_call_graph), RES-1214 (reentrancy_guard),
    // RES-1218 (bounded_blocking + watchdog_feed + sensor_freshness +
    // transaction_commit), RES-1217 (handler-suffix), and RES-1228
    // (rate_limit_static).
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_low_pri_fn = stmts.iter().any(|s| {
        if let Node::Function { name, .. } = &s.node {
            LOW_PRI_PREFIXES.iter().any(|p| name.starts_with(*p))
        } else {
            false
        }
    });
    if !has_low_pri_fn {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        if !LOW_PRI_PREFIXES.iter().any(|p| fname.starts_with(*p)) {
            return;
        }
        let mut raw_lock_seen = false;
        let mut pi_seen = false;
        visit(body, &mut |n| {
            if let Node::CallExpression { function, .. } = n {
                if let Node::Identifier { name, .. } = function.as_ref() {
                    if RAW_LOCKS.contains(&name.as_str()) {
                        raw_lock_seen = true;
                    }
                    if PI_VARIANTS.contains(&name.as_str()) {
                        pi_seen = true;
                    }
                }
            }
        });
        if raw_lock_seen && !pi_seen {
            eprintln!(
                "warning: low-priority fn '{fname}' acquires a non-PI lock — \
                 will cause priority inversion. Use pi_lock/pi_acquire/with_priority_inherit"
            );
        }
    });
    Ok(())
}
