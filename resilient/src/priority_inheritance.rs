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
