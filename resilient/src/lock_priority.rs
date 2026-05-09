//! Feature 34/50 — Static Lock Ordering by Priority.
//!
//! `#[lock_priority(N)]` on a function declares it acquires a lock
//! at priority N. The compiler walks the call graph and rejects any
//! sequence that acquires a lower-priority lock while holding a
//! higher one — preventing classical priority inversion.
//!
//! This complements the existing `lock_ordering` Ralph-Loop lint by
//! making priority a static attribute rather than inferring it from
//! call patterns.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct PrioritySpec {
    pub priority: u32,
}

pub fn collect() -> HashMap<String, PrioritySpec> {
    let attrs = crate::feature_attrs::find_kind("lock_priority");
    let mut out = HashMap::new();
    for (item, rec) in attrs {
        let raw = rec.args.trim();
        if let Ok(n) = raw.parse() {
            out.insert(item, PrioritySpec { priority: n });
        }
    }
    out
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let priorities = collect();
    if priorities.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if let Some(spec) = priorities.get(name) {
                let mut violations = Vec::new();
                walk_priorities(body, spec.priority, &priorities, &mut violations);
                if let Some(v) = violations.into_iter().next() {
                    return Err(format!(
                        "{}:0:0: error: priority inversion in `{}`: holds priority {} but calls `{}` at priority {}",
                        source_path, name, spec.priority, v.0, v.1
                    ));
                }
            }
        }
    }
    Ok(())
}

fn walk_priorities(
    node: &Node,
    holding: u32,
    table: &HashMap<String, PrioritySpec>,
    out: &mut Vec<(String, u32)>,
) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if let Some(p) = table.get(name) {
                    if p.priority < holding {
                        out.push((name.clone(), p.priority));
                    }
                }
            }
            for a in arguments {
                walk_priorities(a, holding, table, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_priorities(s, holding, table, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_priorities(e, holding, table, out),
        Node::LetStatement { value, .. } => walk_priorities(value, holding, table, out),
        Node::ExpressionStatement { expr, .. } => walk_priorities(expr, holding, table, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn high_calling_low_is_inversion() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "high",
            crate::feature_attrs::AttrRecord {
                name: "lock_priority".into(),
                args: "5".into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "low",
            crate::feature_attrs::AttrRecord {
                name: "lock_priority".into(),
                args: "2".into(),
                line: 0,
            },
        );
        let src = r#"
            fn low(int x) { return x; }
            fn high(int x) { return low(x); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }
}
