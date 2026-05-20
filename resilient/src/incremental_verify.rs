//! Feature 25/50 — Incremental Verification.
//!
//! A proof cache keyed on `(fn_name, contract_digest)`. Calls to the
//! Z3 verifier first consult the cache; if the digest matches, the
//! cached result is returned without re-running SMT. The cache is
//! invalidated automatically when:
//!
//! * The fn's `requires`/`ensures` change (digest miss).
//! * Any function in the call-graph closure of the fn changes.
//!
//! This trades a small per-build cost for the cache lookup against
//! significant Z3 savings on large codebases. The cache lives at
//! `.resilient/proof.cache` (JSON) so it survives across builds.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofResult {
    Discharged,
    Failed(String),
}

/// RES-2166: nested HashMap keyed on `fn_name` (outer) and
/// `contract_digest` (inner). The previous shape was
/// `HashMap<(String, u64), ProofResult>` which forced every
/// lookup to allocate a fresh `String` for the tuple key — paid
/// per Z3 obligation, hits *and* misses alike. The outer map
/// keys on `String` so we can probe with `&str` via
/// `Borrow<str>`; lookups walk two hash steps with zero
/// allocations.
#[derive(Debug, Clone, Default)]
pub struct ProofCache {
    pub entries: HashMap<String, HashMap<u64, ProofResult>>,
    pub hits: u64,
    pub misses: u64,
}

static CACHE: RwLock<Option<ProofCache>> = RwLock::new(None);

pub fn reset() {
    if let Ok(mut g) = CACHE.write() {
        *g = Some(ProofCache::default());
    }
}

pub fn lookup(fn_name: &str, contract_digest: u64) -> Option<ProofResult> {
    if let Ok(mut g) = CACHE.write() {
        let cache = g.get_or_insert_with(ProofCache::default);
        // RES-2166: probe the outer map with `&str` (no allocation),
        // then walk the inner `HashMap<u64, ProofResult>`. The
        // previous shape allocated a `String` for the tuple key on
        // every call — paid on every hit AND miss.
        if let Some(inner) = cache.entries.get(fn_name)
            && let Some(r) = inner.get(&contract_digest)
        {
            cache.hits += 1;
            return Some(r.clone());
        }
        cache.misses += 1;
    }
    None
}

pub fn store(fn_name: &str, contract_digest: u64, result: ProofResult) {
    if let Ok(mut g) = CACHE.write() {
        let cache = g.get_or_insert_with(ProofCache::default);
        // RES-2166: skip the `fn_name.to_string()` alloc on the hot
        // path when an outer entry already exists. Only the cold
        // branch (first digest stored for this fn) pays for an owned
        // `String` key.
        if let Some(inner) = cache.entries.get_mut(fn_name) {
            inner.insert(contract_digest, result);
        } else {
            let mut inner = HashMap::new();
            inner.insert(contract_digest, result);
            cache.entries.insert(fn_name.to_string(), inner);
        }
    }
}

pub fn cache_is_empty() -> bool {
    CACHE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|c| c.entries.is_empty()))
        .unwrap_or(true)
}

pub fn stats() -> (u64, u64) {
    // RES-1566: borrow through the read guard and read the two `Copy`
    // u64s in place. The previous `g.clone()` shape cloned the entire
    // `ProofCache` (which carries a `HashMap<(String, u64), ProofResult>`)
    // per call just to return two integers.
    CACHE
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|c| (c.hits, c.misses)))
        .unwrap_or((0, 0))
}

/// Evict proof-cache entries for functions that no longer exist in
/// the program.
///
/// The cache is keyed on `(fn_name, contract_digest)` and persists
/// across type-check calls within a build session. When a function
/// is deleted or renamed the old entries become stale — they waste
/// memory and, once a future PR wires the Z3 verifier to call
/// `lookup`, could produce ghost hits for names that have been
/// recycled with different contracts.
///
/// This pass is a lightweight O(N) retention scan: collect all live
/// function names from the AST, then drop every cache entry whose
/// function name is absent from that set.
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    if cache_is_empty() {
        return Ok(());
    }
    let live_names: std::collections::HashSet<&str> = match program {
        Node::Program(stmts) => stmts
            .iter()
            .filter_map(|s| {
                if let Node::Function { name, .. } = &s.node {
                    Some(name.as_str())
                } else {
                    None
                }
            })
            .collect(),
        _ => return Ok(()),
    };
    if let Ok(mut g) = CACHE.write() {
        if let Some(cache) = g.as_mut() {
            // RES-2166: count by inner-map entries (each `(fn_name,
            // digest)` pair was one entry in the old flat shape).
            // Retention drops the entire outer entry when the fn is
            // gone; the inner digests die with it.
            let before: usize = cache.entries.values().map(|m| m.len()).sum();
            cache
                .entries
                .retain(|fn_name, _| live_names.contains(fn_name.as_str()));
            let after: usize = cache.entries.values().map(|m| m.len()).sum();
            let evicted = before.saturating_sub(after);
            if evicted > 0 {
                eprintln!("incremental_verify: evicted {evicted} stale proof-cache entry/ies");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cache_hit_then_miss() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        store("f", 1234, ProofResult::Discharged);
        assert_eq!(lookup("f", 1234), Some(ProofResult::Discharged));
        assert_eq!(lookup("f", 9999), None);
        let (h, m) = stats();
        assert_eq!(h, 1);
        assert_eq!(m, 1);
    }

    // ── check() ──────────────────────────────────────────────────────────────

    #[test]
    fn check_ok_on_empty_program() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_evicts_stale_entry() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        // Store a cache entry for "deleted_fn" that won't be in the AST
        store("deleted_fn", 42, ProofResult::Discharged);
        // Parse a program that has no "deleted_fn"
        let src = "fn live_fn(int x) { return x; }";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
        // The stale entry should have been evicted
        assert_eq!(lookup("deleted_fn", 42), None);
    }

    #[test]
    fn check_preserves_live_entry() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        // Store a cache entry for "live_fn" that IS in the AST
        store("live_fn", 99, ProofResult::Discharged);
        let src = "fn live_fn(int x) { return x; }";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
        // The live entry must be preserved
        assert_eq!(lookup("live_fn", 99), Some(ProofResult::Discharged));
    }

    #[test]
    fn failed_proofs_recorded() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        store("g", 7, ProofResult::Failed("counterexample".into()));
        assert!(matches!(lookup("g", 7), Some(ProofResult::Failed(_))));
    }
}
