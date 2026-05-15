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
    // RES-1485: run validation before `install` so we can move
    // `set` into install rather than cloning. The previous shape
    // did `install(set.clone())` ahead of the for-loop just so
    // `&set` could still be iterated. Same shape as RES-1481 for
    // `derives::check`. The warnings emitted during validation
    // are eprintln only — no early return — so the install always
    // runs at the same point in the success path.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // Validation: a recursive type must syntactically reference itself.
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
    install(set);
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

    #[test]
    fn collect_returns_empty_without_attribute() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let set = collect();
        assert!(
            set.is_empty(),
            "collect() must return empty set when no #[recursive] attributes exist"
        );
    }

    #[test]
    fn is_recursive_false_before_install() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Install an empty set — is_recursive should return false for any name.
        install(collect());
        assert!(
            !is_recursive("AnyType"),
            "is_recursive must be false when no types are registered"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "check() must return Ok when no #[recursive] attributes exist"
        );
    }

    #[test]
    fn multiple_recursive_types_all_registered() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        for name in &["Node", "Tree", "List"] {
            crate::feature_attrs::record(
                name,
                crate::feature_attrs::AttrRecord {
                    name: "recursive".into(),
                    args: String::new(),
                    line: 0,
                },
            );
        }
        install(collect());
        assert!(is_recursive("Node"));
        assert!(is_recursive("Tree"));
        assert!(is_recursive("List"));
        assert!(!is_recursive("Array"));
        crate::feature_attrs::reset();
    }
}
