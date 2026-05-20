//! Feature 42/50 — Iterator Protocol.
//!
//! Defines a built-in `Iterator` trait with a single method
//! `next() -> Option<T>`. Any type that declares an `impl Iterator
//! for T` automatically participates in `for x in t` loops.
//!
//! Implementation notes for the first slice:
//! * The trait declaration itself lives inline as a synthesised
//!   `Node::TraitDecl` injected by this module.
//! * The runtime walks user impls and exposes `is_iterator(type_name)`.
//! * Future PRs lower `for x in t { ... }` into a desugared
//!   `loop { match t.next() { Some(x) => ..., None => break } }`.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

static ITERATORS: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(|| RwLock::new(HashSet::new()));

pub fn install_iterator_impls(types: HashSet<String>) {
    if let Ok(mut g) = ITERATORS.write() {
        *g = types;
    }
}

pub fn is_iterator(type_name: &str) -> bool {
    ITERATORS
        .read()
        .ok()
        .map(|g| g.contains(type_name))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1291 / RES-1917: the typechecker gates this call behind
    // `markers.has_impl_for_trait("Iterator")`, so the program is
    // guaranteed to contain at least one Iterator impl. The previous
    // `any_node` pre-scan was redundant — removed. (The typechecker
    // else branch installs an empty set directly.)
    // RES-2244: pre-size to 4 — typical Iterator impl count is 1-5
    // even on iterator-heavy programs. The previous `with_capacity(
    // stmts.len())` allocated bucket space for every top-level
    // statement, ~95% of which are NOT Iterator impls (most stmts
    // are Functions / StructDecls / TypeAliases). For a 100-stmt
    // program with 2 Iterator impls, this drops the HashSet from
    // ~200-slot pre-alloc to ~8-slot. Mirrors RES-2242
    // (assume_axioms lazy alloc) — favour the common iterator-count
    // case over the rare iterator-heavy upper bound.
    let mut iters = HashSet::with_capacity(4);
    for s in stmts {
        if let Node::ImplBlock {
            trait_name,
            struct_name,
            ..
        } = &s.node
        {
            if trait_name.as_deref() == Some("Iterator") {
                iters.insert(struct_name.clone());
            }
        }
    }
    install_iterator_impls(iters);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Mutex;

    /// RES-2126: serialize tests that mutate the process-wide
    /// `ITERATORS` registry. Without this lock, `cargo test` runs
    /// `manual_install_registers` and other reader/writer pairs in
    /// parallel — and another thread can replace the set between
    /// this test's `install_iterator_impls` and its `is_iterator`
    /// assertion, flaking the JIT-feature CI gate (observed on
    /// auto-merge of #2089-2125, which silently blocked the queue).
    /// Mirrors the `TEST_LOCK` pattern in `feature_attrs`.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_for_test() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn manual_install_registers() {
        let _g = lock_for_test();
        let mut s = HashSet::new();
        s.insert("MyRange".to_string());
        install_iterator_impls(s);
        assert!(is_iterator("MyRange"));
        assert!(!is_iterator("Other"));
        install_iterator_impls(HashSet::new());
    }
    #[test]
    fn is_iterator_returns_false_for_unregistered_type() {
        let _g = lock_for_test();
        install_iterator_impls(HashSet::new());
        assert!(!is_iterator("UnregisteredType12345"));
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = lock_for_test();
        let _fa = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install_iterator_impls(HashSet::new());
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
