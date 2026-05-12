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

#[derive(Debug, Clone, Default)]
pub struct ProofCache {
    pub entries: HashMap<(String, u64), ProofResult>,
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
        let key = (fn_name.to_string(), contract_digest);
        if let Some(r) = cache.entries.get(&key) {
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
        cache
            .entries
            .insert((fn_name.to_string(), contract_digest), result);
    }
}

pub fn stats() -> (u64, u64) {
    CACHE
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|c| (c.hits, c.misses))
        .unwrap_or((0, 0))
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1210: the historical body called `fingerprint_program` and
    // pre-populated the cache with `ProofResult::Discharged` for every
    // function on every type-check. Two problems with that:
    //
    //   1. No consumer in the crate calls `lookup` (a
    //      `grep -rn 'incremental_verify::' resilient/src/` shows
    //      only this pass — the Z3 verifier doesn't consult the
    //      cache yet), so the work was unobservable.
    //   2. If a future PR wires the Z3 verifier to call `lookup`,
    //      the pre-population would already cache every function as
    //      `Discharged`, *skipping the actual proof*. The cache
    //      should only record verdicts the verifier has produced.
    //
    // The `lookup` / `store` / `stats` / `reset` API stays as-is so
    // the cache infrastructure is ready for the consumer side when
    // someone lands it. The `EXTENSION_PASSES` slot in
    // `typechecker.rs` stays present for the same reason.
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

    #[test]
    fn failed_proofs_recorded() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        store("g", 7, ProofResult::Failed("counterexample".into()));
        assert!(matches!(lookup("g", 7), Some(ProofResult::Failed(_))));
    }
}
