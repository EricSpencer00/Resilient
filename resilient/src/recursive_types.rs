//! Feature 18/50 — Recursive Types via Boxed Self-Reference.
//!
//! `#[recursive]` on a struct or enum signals that the type may
//! contain a boxed reference to itself (linked lists, trees, state
//! machines, AST nodes). Without this, you can't represent any
//! recursive data structure in Resilient — a fundamental gap.
//!
//! This first slice records the recursive declaration in a registry
//! and provides a `is_recursive(type_name)` query the runtime uses
//! to allocate boxed cells. Lowering the actual indirection at code-
//! gen time is a downstream PR that wires into the VM's heap.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;
use std::sync::RwLock;

static RECURSIVE_TYPES: RwLock<Option<HashSet<String>>> = RwLock::new(None);

pub fn collect() -> HashSet<String> {
    let attrs = crate::feature_attrs::find_kind("recursive");
    attrs.into_iter().map(|(item, _)| item).collect()
}

pub fn install(set: HashSet<String>) {
    if let Ok(mut g) = RECURSIVE_TYPES.write() {
        *g = Some(set);
    }
}

pub fn is_recursive(type_name: &str) -> bool {
    RECURSIVE_TYPES
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|s| s.contains(type_name))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let set = collect();
    // RES-1244: fast-reject. The "does it reference itself?"
    // diagnostic only fires for struct names in `set`, i.e. those
    // annotated `#[recursive]`. When no such attribute exists in
    // the program (the overwhelming common case), `set` is empty
    // and the per-statement loop produces no output.
    //
    // RES-1308: also gate `install` on the non-empty case. The
    // historical wiring called `install(set.clone())` before the
    // early-out, burning a RwLock write per compile and creating
    // the wipe-on-empty test race documented in RES-1302.
    if set.is_empty() {
        return Ok(());
    }
    install(set.clone());
    // Validation: a recursive type must syntactically reference itself.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::StructDecl { name, fields, .. } = &s.node {
            if set.contains(name) {
                let self_ref = fields.iter().any(|f| field_type_contains(f, name));
                if !self_ref {
                    eprintln!(
                        "warning: `#[recursive] struct {}` does not reference itself in any field",
                        name
                    );
                }
            }
        }
    }
    Ok(())
}

fn field_type_contains(field: &(String, String), name: &str) -> bool {
    field.0.contains(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_recursive_marker() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "Tree",
            crate::feature_attrs::AttrRecord {
                name: "recursive".into(),
                args: String::new(),
                line: 0,
            },
        );
        install(collect());
        assert!(is_recursive("Tree"));
        assert!(!is_recursive("Other"));
        crate::feature_attrs::reset();
    }
}
