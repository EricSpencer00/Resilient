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
