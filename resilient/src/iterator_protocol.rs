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
    // RES-1291: fast-reject. The for-loop below filters every top-
    // level statement looking for an `ImplBlock` whose trait_name is
    // `"Iterator"`. Programs without any Iterator impl produce an
    // empty set; the loop is dead work for them. Pre-scan with the
    // early-terminating `any_node` (RES-1238) and short-circuit to
    // an empty install when no matching ImplBlock exists. We still
    // install an empty set so a prior program's iterator-impl
    // registration doesn't leak into this compilation (the
    // process-global ITERATORS otherwise retains stale entries).
    let has_iterator_impl = crate::uniqueness_walk::any_node(program, |n| match n {
        Node::ImplBlock { trait_name, .. } => trait_name.as_deref() == Some("Iterator"),
        _ => false,
    });
    if !has_iterator_impl {
        install_iterator_impls(HashSet::new());
        return Ok(());
    }
    let mut iters = HashSet::new();
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

    #[test]
    fn manual_install_registers() {
        let mut s = HashSet::new();
        s.insert("MyRange".to_string());
        install_iterator_impls(s);
        assert!(is_iterator("MyRange"));
        assert!(!is_iterator("Other"));
        install_iterator_impls(HashSet::new());
    }
}
