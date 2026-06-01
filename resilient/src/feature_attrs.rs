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
//! | `#[secret]` / `#[public]` / `#[declassify]` | `info_flow` | Information-flow tags |
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
use std::sync::atomic::{AtomicBool, Ordering};
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

/// RES-1303: dual-index registry. `by_item` keeps the historic
/// item-keyed view that `snapshot()` exposes; `by_kind` is the
/// reverse index `find_kind` reads from for O(matching-entries)
/// lookup instead of O(total-registry-entries). Both indexes are
/// maintained transactionally by `record` / `reset`.
#[derive(Debug, Default, Clone)]
struct Registry {
    by_item: AttrMap,
    by_kind: HashMap<String, Vec<(String, AttrRecord)>>,
}

static ATTR_REGISTRY: RwLock<Option<Registry>> = RwLock::new(None);

/// RES-1374: cheap fast-reject mirror of "registry contains at least
/// one recorded attribute." `find_kind` is called ~31 times per
/// typecheck (one per analysis pass that owns a feature kind). The
/// common case is a program with zero recognized attributes, in which
/// every call would otherwise acquire the `ATTR_REGISTRY` `RwLock`
/// just to discover the registry is empty. An atomic-bool gate skips
/// the lock acquire entirely on the empty path.
///
/// Maintained transactionally:
/// - `record` stores `true` (Release) after its registry write.
/// - `reset` stores `false` (Release) so post-reset `find_kind` calls
///   bypass the lock until the next `record`.
/// - `find_kind` loads (Acquire) before optionally acquiring the lock.
///
/// The Release / Acquire pair ensures any registry write made before
/// the `record`-side store is visible to a `find_kind` that took the
/// lock after observing `true`.
static ATTR_REGISTRY_HAS_ENTRIES: AtomicBool = AtomicBool::new(false);

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
    // RES-2438: if the atomic flag is already false, no `record()` call
    // has fired since the last reset (or since process start). The
    // registry is already empty (or `None`) — skip the write-lock
    // acquisition and the `Registry::default()` allocation entirely.
    // Same fast-reject pattern that `find_kind` uses (RES-1374).
    if !ATTR_REGISTRY_HAS_ENTRIES.load(Ordering::Acquire) {
        return;
    }
    if let Ok(mut g) = ATTR_REGISTRY.write() {
        *g = Some(Registry::default());
    }
    // RES-1374: registry is empty after reset; clear the fast-reject
    // gate so `find_kind` bypasses the lock until the next `record`.
    ATTR_REGISTRY_HAS_ENTRIES.store(false, Ordering::Release);
}

/// Record an attribute against an item name. Idempotent if called
/// before [`reset`] — multiple identical entries are allowed (each
/// feature module decides whether duplicates are an error).
///
/// RES-2088: the dual-index write previously called `item_name.to_string()`
/// twice (once as `by_item`'s entry key, once as the tuple value pushed
/// into `by_kind`) and `record.name.clone()` once for `by_kind`'s entry
/// key — three `String` allocations per call, two of which were thrown
/// away whenever the relevant key was already present. Each typecheck
/// records ~50 annotated items × ~21 known attribute kinds, with
/// `record.name` repeating heavily across items.
///
/// New shape: probe each map with `get_mut(&str)` first (`String: Borrow<str>`
/// makes the borrow lookup free), only allocate when the key is genuinely
/// absent, and move `record` into the second map's tuple to skip the
/// trailing clone. Worst case one allocation per call (the value-side
/// `item_name.to_string()` that lives in the `by_kind` tuple), down from
/// three.
pub fn record(item_name: &str, record: AttrRecord) {
    if let Ok(mut g) = ATTR_REGISTRY.write() {
        let reg = g.get_or_insert_with(Registry::default);

        // by_item: borrow for the lookup; allocate the key only if absent.
        match reg.by_item.get_mut(item_name) {
            Some(entries) => entries.push(record.clone()),
            None => {
                reg.by_item
                    .insert(item_name.to_string(), vec![record.clone()]);
            }
        }

        // by_kind: same trick on the kind key. Moves `record` into the
        // tuple so the trailing `record.clone()` allocation disappears.
        match reg.by_kind.get_mut(record.name.as_str()) {
            Some(entries) => entries.push((item_name.to_string(), record)),
            None => {
                let kind = record.name.clone();
                reg.by_kind
                    .insert(kind, vec![(item_name.to_string(), record)]);
            }
        }
    }
    // RES-1374: signal "registry now has entries" after the write
    // commits. Release ordering pairs with the Acquire load in
    // `find_kind` so the registry write is happens-before-visible to
    // any reader that takes the lock after observing `true`.
    ATTR_REGISTRY_HAS_ENTRIES.store(true, Ordering::Release);
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
        .and_then(|g| g.as_ref().map(|r| r.by_item.clone()))
        .unwrap_or_default()
}

/// Lookup all attributes of a given kind across all items. Returns
/// `(item_name, AttrRecord)` pairs.
///
/// RES-1303: O(matching-entries) lookup via the kind-indexed reverse
/// map. The previous implementation (RES-1224) walked every
/// `(item, attrs)` pair in the item-keyed map filtering by
/// `a.name == kind`. With ~31 analysis passes calling `find_kind`
/// per typecheck and a registry that accumulates entries across
/// parses, that cost scaled linearly with the *total* records in
/// the registry regardless of whether the queried kind existed.
/// Maintaining a parallel `by_kind` index in `record` collapses each
/// lookup to a single `HashMap::get` + `Vec::clone` of only the
/// matching entries — pays only for what the caller consumes.
///
/// The lock is held read-only so concurrent readers (parallel tests)
/// don't serialise; insertion order within a kind is preserved.
pub fn find_kind(kind: &str) -> Vec<(String, AttrRecord)> {
    // RES-1374: fast-reject before touching the `RwLock` when the
    // registry has no recorded attributes. With ~31 EXTENSION_PASSES
    // callers per typecheck and the common case being zero recorded
    // attributes, skipping the read-lock acquire on the empty path
    // is a measurable win — each call drops from a ~µs lock op to an
    // atomic load. Acquire ordering pairs with the Release store in
    // `record`.
    if !ATTR_REGISTRY_HAS_ENTRIES.load(Ordering::Acquire) {
        return Vec::new();
    }
    let Ok(g) = ATTR_REGISTRY.read() else {
        return Vec::new();
    };
    let Some(reg) = g.as_ref() else {
        return Vec::new();
    };
    reg.by_kind.get(kind).cloned().unwrap_or_default()
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
            // RES-2824: information-flow laundering boundary.
            | "declassify"
            // RES-2825: semantic non-interference proof obligation.
            | "noninterference"
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
            // RES-2592: tail call optimization enforcement.
            | "must_tail_call"
            // RES-2659: mutual tail call optimization.
            | "mutual_tail_call"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // RES-1280: serialise registry-touching tests against the
    // *binary-wide* `TEST_LOCK` (via `lock_for_test()`), not a
    // module-local mutex.
    //
    // RES-1132 added a private `serial_lock()` here that used its own
    // fresh `Mutex`. That serialised the two in-module tests against
    // each other, but not against the dozens of tests in other
    // modules (`refinement_types`, `session_types`, `lock_priority`,
    // `info_flow`, `wcet_contracts`, etc.) that drive the same
    // `ATTR_REGISTRY` through `lock_for_test()`. Result: another
    // module's `reset()` could still wipe this test's `record` between
    // the `record` and the `find_kind`, intermittently failing the
    // `assert_eq!(found.len(), 1)` below with `0`. Reusing the public
    // `TEST_LOCK` makes every registry-touching test in the binary
    // serial.

    #[test]
    fn record_and_lookup() {
        let _g = lock_for_test();
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
        let _g = lock_for_test();
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
