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
    // RES-1754: pre-size to attrs.len() — at most one insert per
    // attribute record (conditional on parse success).
    let mut out = HashMap::with_capacity(attrs.len());
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
                // RES-1569: short-circuit on first inversion. The previous
                // shape pushed every violation into a `Vec<(String, u32)>`
                // and then took `into_iter().next()` — every violation
                // past the first was wasted plus the Vec allocation
                // itself. Returning `Option` from the walk avoids both.
                if let Some((callee, callee_pri)) =
                    walk_priorities(body, spec.priority, &priorities)
                {
                    return Err(format!(
                        "{}:0:0: error: priority inversion in `{}`: holds priority {} but calls `{}` at priority {}",
                        source_path, name, spec.priority, callee, callee_pri
                    ));
                }
            }
        }
    }
    Ok(())
}

// RES-1539: borrow the callee name through to the diagnostic site
// rather than cloning. The walker's only consumer (the priority-
// inversion `format!` in `check`) just `Display`s the returned name
// — `&str` works the same as `String`, and the borrow chain is
// sound: `name` lives in `Node::Identifier` inside the program AST,
// which outlives the walk + format call. Same pattern as RES-1500 /
// RES-1525 etc.
fn walk_priorities<'a>(
    node: &'a Node,
    holding: u32,
    table: &HashMap<String, PrioritySpec>,
) -> Option<(&'a str, u32)> {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if let Some(p) = table.get(name) {
                    if p.priority < holding {
                        return Some((name.as_str(), p.priority));
                    }
                }
            }
            for a in arguments {
                if let Some(v) = walk_priorities(a, holding, table) {
                    return Some(v);
                }
            }
            None
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                if let Some(v) = walk_priorities(s, holding, table) {
                    return Some(v);
                }
            }
            None
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_priorities(e, holding, table),
        Node::LetStatement { value, .. } => walk_priorities(value, holding, table),
        Node::ExpressionStatement { expr, .. } => walk_priorities(expr, holding, table),
        _ => None,
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
