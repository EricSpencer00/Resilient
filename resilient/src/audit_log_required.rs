//! Ralph-Loop Uniqueness #19 — audit-log-required mutations.
//!
//! Compliance regimes (HIPAA, SOX, PCI) require certain mutations to
//! produce an audit-log entry. Today this is enforced by code review and
//! integration testing. Database triggers can do it at runtime; no
//! programming language *requires*, at compile time, that a write to a
//! field tagged "auditable" be paired with a logging call.
//!
//! Resilient enforces by struct-field name: any field assignment whose
//! field name starts with `audited_` or ends with `_audited` must, in
//! the *same function body*, be paired with a call to `audit_log` /
//! `journal` / `record_event` / `emit_audit`. Otherwise we warn.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function, visit};

const AUDIT_FNS: &[&str] = &["audit_log", "journal", "record_event", "emit_audit"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1254: fast-reject. The per-function `visit(body)` walk only
    // does anything when it finds a `FieldAssignment` whose field name
    // starts with `audited_` or ends with `_audited`. For programs
    // that have no such fields (the overwhelming majority of test
    // inputs and the entire `examples/` tree), every per-function
    // visit produces nothing. Pre-scan the whole program once via
    // `any_node` (RES-1238 made this early-terminating) and skip the
    // pass entirely when no audited field exists.
    let has_audited_field = any_node(program, |n| match n {
        Node::FieldAssignment { field, .. } => {
            field.starts_with("audited_") || field.ends_with("_audited")
        }
        _ => false,
    });
    if !has_audited_field {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        let mut has_audited_write = false;
        visit(body, &mut |n| {
            if let Node::FieldAssignment { field, .. } = n {
                if field.starts_with("audited_") || field.ends_with("_audited") {
                    has_audited_write = true;
                }
            }
        });
        if !has_audited_write {
            return;
        }
        let logged = any_node(body, |n| match n {
            Node::CallExpression { function, .. } => match function.as_ref() {
                Node::Identifier { name, .. } => AUDIT_FNS.contains(&name.as_str()),
                _ => false,
            },
            _ => false,
        });
        if !logged {
            eprintln!(
                "warning: function '{fname}' writes to an audited field but never \
                 calls audit_log()/journal()/record_event() — compliance violation"
            );
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_audited_field_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn audit_fns_include_expected_names() {
        assert!(AUDIT_FNS.contains(&"audit_log"));
        assert!(AUDIT_FNS.contains(&"journal"));
    }
}
