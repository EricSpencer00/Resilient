//! Feature 33/50 — Atomic Types.
//!
//! `#[atomic]` on a `static let` binding marks it as a lock-free
//! shared cell. The runtime backs it by a Rust `AtomicI64` and
//! exposes ordering-aware accessor builtins:
//!
//! * `atomic_load(name) -> int`
//! * `atomic_store(name, value)`
//! * `atomic_fetch_add(name, delta) -> int`
//!
//! The first slice ships the registry of atomic names so the runtime
//! and typechecker can validate usage.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::RwLock;
use std::sync::atomic::{AtomicI64, Ordering};

#[derive(Debug, Default)]
struct AtomicRegistry {
    cells: HashMap<String, AtomicI64>,
}

static REGISTRY: RwLock<Option<AtomicRegistry>> = RwLock::new(None);

pub fn collect_names() -> Vec<String> {
    crate::feature_attrs::find_kind("atomic")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

fn ensure() {
    if let Ok(mut g) = REGISTRY.write() {
        if g.is_none() {
            *g = Some(AtomicRegistry::default());
        }
    }
}

pub fn declare(name: &str, initial: i64) {
    ensure();
    if let Ok(mut g) = REGISTRY.write() {
        let r = g.get_or_insert_with(AtomicRegistry::default);
        r.cells.insert(name.to_string(), AtomicI64::new(initial));
    }
}

pub fn load(name: &str) -> Option<i64> {
    REGISTRY.read().ok().and_then(|g| {
        g.as_ref()
            .and_then(|r| r.cells.get(name).map(|a| a.load(Ordering::SeqCst)))
    })
}

pub fn store(name: &str, value: i64) -> bool {
    if let Ok(g) = REGISTRY.read() {
        if let Some(r) = g.as_ref() {
            if let Some(a) = r.cells.get(name) {
                a.store(value, Ordering::SeqCst);
                return true;
            }
        }
    }
    false
}

pub fn fetch_add(name: &str, delta: i64) -> Option<i64> {
    REGISTRY.read().ok().and_then(|g| {
        g.as_ref().and_then(|r| {
            r.cells
                .get(name)
                .map(|a| a.fetch_add(delta, Ordering::SeqCst))
        })
    })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    for n in collect_names() {
        declare(&n, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_add_is_atomic() {
        declare("counter", 0);
        let prev = fetch_add("counter", 5);
        assert_eq!(prev, Some(0));
        let prev = fetch_add("counter", 3);
        assert_eq!(prev, Some(5));
        assert_eq!(load("counter"), Some(8));
    }

    #[test]
    fn store_overwrites() {
        declare("flag", 0);
        store("flag", 42);
        assert_eq!(load("flag"), Some(42));
    }
}
