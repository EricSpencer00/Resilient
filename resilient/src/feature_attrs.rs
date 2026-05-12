//! Shared attribute registry for the 50-feature missing-language-features pass.
//!
//! Several of the new features land as `#[name(args)]` attributes on
//! functions, structs, or modules. Rather than each feature module
//! independently extending the parser, they share this registry: the
//! parser entry point in [`crate::cfg_attr`] dispatches recognized
//! non-`cfg` attribute names here, the attribute and its arguments are
//! recorded keyed on the following item's name, and downstream analysis
//! passes read from the registry during the `<EXTENSION_PASSES>` stage
//! of typechecking.
//!
//! This module owns *only* the storage and the parser-side hook. Each
//! feature module (`refinement_types`, `wcet_contracts`, ...) reads its
//! own attribute kind from here and emits its own diagnostics.
//!
//! ## Recognized attributes (initial slice)
//!
//! | Attribute | Owner module | Purpose |
//! |---|---|---|
//! | `#[refinement(...)]` | `refinement_types` | Type-level constraint |
//! | `#[typestate(...)]` | `typestate_types` | Lifecycle states |
//! | `#[wcet(cycles=N)]` | `wcet_contracts` | Worst-case execution time |
//! | `#[power(uj=N)]` | `power_contracts` | Energy budget |
//! | `#[stack(bytes=N)]` | `stack_contracts` | Stack-depth budget |
//! | `#[no_alloc]` | `no_alloc_cert` | Allocation-freedom certificate |
//! | `#[ghost]` | `ghost_types` | Specification-only code |
//! | `#[derive(...)]` | `derives` | Auto-derive trait impls |
//! | `#[mmio(base=...)]` | `mmio_regmap` | Typed register map |
//! | `#[stable(since=...)]` | `anti_regression` | Behavioral lock-in |
//! | `#[intent(...)]` | `intent_blocks` | High-level specification |
//! | `#[crash_only_cert]` | `crash_only_cert` | Crash-only proof |
//! | `#[property_test]` | `property_tests` | Auto-generated property tests |
//! | `#[const_fn]` | `const_fn` | Compile-time evaluation |
//! | `#[async_fn]` | `async_await` | Async function |
//! | `#[secret]` / `#[public]` | `info_flow` | Information-flow tags |
//! | `#[phantom]` | `phantom_types` | Type-erased marker |
//! | `#[atomic]` | `atomic_types` | Lock-free primitive |
//! | `#[lock_priority(N)]` | `lock_priority` | Static lock ordering |
//! | `#[peripheral]` | `hw_state_machine` | Hardware lifecycle type |
//! | `#[autopilot]` | `autopilot` | Mark for safety audit |
//!
//! Attributes that do not match a known kind fall through to
//! `cfg_attr`'s existing "unknown attribute" error path.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, RwLock};

/// One recorded attribute application. The arguments string is kept
/// raw (as written between the parens) so each feature module can
/// parse it according to its own surface syntax. This avoids growing
/// a shared parser that has to know every feature's grammar.
#[derive(Debug, Clone)]
pub struct AttrRecord {
    /// Attribute name, e.g. `"refinement"`, `"wcet"`. Without the `#[]`.
    pub name: String,
    /// Raw text between the parentheses, or empty for `#[name]` flag-only.
    pub args: String,
    /// Source line where the attribute was written (best-effort).
    #[allow(dead_code)]
    pub line: usize,
}

/// Keyed by the *target item name* (function name or struct name).
/// One item may carry several attributes — they're appended in order.
type AttrMap = HashMap<String, Vec<AttrRecord>>;

static ATTR_REGISTRY: RwLock<Option<AttrMap>> = RwLock::new(None);

/// 50-feature pass: any test that mutates the global attribute
/// registry (i.e. calls `record` / `reset` / `find_kind` against
/// state it just installed) must hold this mutex for the duration
/// of the test. The registry is process-wide; without this lock,
/// `cargo test` runs feature tests in parallel and they observe
/// each other's records.
///
/// Usage:
/// ```ignore
/// let _g = crate::feature_attrs::lock_for_test();
/// crate::feature_attrs::reset();
/// crate::feature_attrs::record(...);
/// // ... assertions
/// ```
static TEST_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the test serialisation lock. Poison-tolerant: a panicking
/// test that held the lock leaves it poisoned, but the next test
/// recovers via `into_inner`.
#[allow(dead_code)]
pub fn lock_for_test() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Reset the registry. Must be called at the start of every parse so
/// the LSP / REPL / test harness don't see stale entries from a
/// previous compile.
#[allow(dead_code)]
pub fn reset() {
    if let Ok(mut g) = ATTR_REGISTRY.write() {
        *g = Some(AttrMap::new());
    }
}

/// Record an attribute against an item name. Idempotent if called
/// before [`reset`] — multiple identical entries are allowed (each
/// feature module decides whether duplicates are an error).
pub fn record(item_name: &str, record: AttrRecord) {
    if let Ok(mut g) = ATTR_REGISTRY.write() {
        let map = g.get_or_insert_with(AttrMap::new);
        map.entry(item_name.to_string()).or_default().push(record);
    }
}

/// Snapshot the registry for read-only inspection by an analysis pass.
/// Returns an empty map if nothing has been registered yet.
///
/// RES-1224: `find_kind` no longer goes through this — it reads the
/// registry directly under the `RwLock` to avoid the full-registry
/// clone — but the function is kept public for callers that want the
/// whole map (e.g. the `--audit` reporter and any external integrator).
#[allow(dead_code)]
pub fn snapshot() -> AttrMap {
    ATTR_REGISTRY
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// Lookup all attributes of a given kind across all items. Returns
/// `(item_name, AttrRecord)` pairs.
///
/// RES-1224: read the registry under its `RwLock` without snapshotting.
/// The previous implementation called `snapshot()`, which deep-cloned
/// the whole `HashMap<String, Vec<AttrRecord>>` (and every `AttrRecord`
/// inside, each carrying two owned `String`s), then filtered. With
/// ~25 analysis passes calling `find_kind` per typecheck, every call
/// allocated a full-registry clone on the way to discarding most of
/// it. Walking the map directly and cloning only matching entries
/// keeps the same return shape while allocating only the bytes the
/// caller actually consumes. The lock is held read-only and the walk
/// is bounded by `map.len()`, so concurrent readers (parallel tests)
/// don't serialise.
pub fn find_kind(kind: &str) -> Vec<(String, AttrRecord)> {
    let Ok(g) = ATTR_REGISTRY.read() else {
        return Vec::new();
    };
    let Some(map) = g.as_ref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (item, attrs) in map {
        for a in attrs {
            if a.name == kind {
                out.push((item.clone(), a.clone()));
            }
        }
    }
    out
}

/// The set of attribute names this registry recognises. Membership is
/// checked by `cfg_attr::parse_cfg_attribute` before it would otherwise
/// emit an "unknown attribute" error.
pub fn is_known_attribute(name: &str) -> bool {
    matches!(
        name,
        "refinement"
            | "typestate"
            | "wcet"
            | "power"
            | "stack"
            | "no_alloc"
            | "ghost"
            | "derive"
            | "mmio"
            | "stable"
            | "intent"
            | "crash_only_cert"
            | "property_test"
            | "const_fn"
            | "async_fn"
            | "secret"
            | "public"
            | "phantom"
            | "atomic"
            | "lock_priority"
            | "peripheral"
            | "autopilot"
            | "no_panic"
            | "deadlock_free"
            | "session"
            | "row_poly"
            | "dependent"
            | "recursive"
            | "ghost_fn"
            | "blame"
            | "version"
            | "ai_review_required"
            | "lean_spec"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// RES-1132: `record_and_lookup` and `reset_clears_state` both
    /// drive the global `ATTR_REGISTRY`. cargo runs tests in parallel
    /// by default, so without serialization the two races: A's
    /// `record` can be wiped by B's `reset` between A's `record` and
    /// `find_kind` calls, intermittently failing `record_and_lookup`
    /// with `len = 0`. A per-module mutex held for the entire test
    /// makes the global-state writes serial without affecting any
    /// other test in the binary.
    fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        // Poison-tolerant: if a prior test panicked while holding the
        // lock, the registry isn't corrupted (reset() rebuilds it),
        // so we recover the guard and proceed.
        LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn record_and_lookup() {
        let _g = serial_lock();
        reset();
        record(
            "my_fn",
            AttrRecord {
                name: "wcet".into(),
                args: "cycles=500".into(),
                line: 10,
            },
        );
        let found = find_kind("wcet");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "my_fn");
        assert_eq!(found[0].1.args, "cycles=500");
    }

    #[test]
    fn known_attributes() {
        assert!(is_known_attribute("refinement"));
        assert!(is_known_attribute("wcet"));
        assert!(!is_known_attribute("not_a_real_attribute"));
    }

    #[test]
    fn reset_clears_state() {
        let _g = serial_lock();
        reset();
        record(
            "f",
            AttrRecord {
                name: "ghost".into(),
                args: String::new(),
                line: 1,
            },
        );
        assert_eq!(find_kind("ghost").len(), 1);
        reset();
        assert_eq!(find_kind("ghost").len(), 0);
    }
}
