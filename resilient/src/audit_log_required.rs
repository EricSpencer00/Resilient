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
