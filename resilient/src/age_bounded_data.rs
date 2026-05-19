//! Ralph-Loop Uniqueness #13 — age-bounded data must be refreshed before use.
//!
//! Telemetry, GPS fixes, and battery readings all become useless after a
//! threshold. ROS bag tools and MAVLink stamp every message at runtime;
//! no language enforces, at compile time, that a value annotated with a
//! maximum staleness is refreshed within its window before consumption.
//!
//! Resilient enforces by name. A struct field whose name ends in
//! `_at` / `_taken` / `_obs_at` / `_observed` represents an
//! observation-timestamp; a sibling field whose name does NOT end in
//! `_at` is treated as the value. If a function body reads a value field
//! without first reading its companion `*_at` and using it in a relational
//! expression (a freshness gate), we warn.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1271 / RES-1917: the typechecker gates this call behind
    // `markers.any_field_accessed_with_suffix(&["_at"])`, so the
    // program is guaranteed to contain at least one `*_at` field
    // access. The previous `any_node` pre-scan was redundant —
    // removed.
    for_each_function(program, |fname, _params, body| {
        // RES-1750: pre-size per-fn collections to 8 — typical fn
        // body has a handful of field reads / age comparisons.
        //
        // RES-2054: borrow target/field names as `&str` from the AST
        // instead of cloning into owned Strings. `visit` exposes
        // lifetime-tied `&'a Node` references so the collected
        // entries can borrow into the body for the duration of the
        // walk. Also drops the unused field slot from `compares_age`
        // — only the target name is consulted by the freshness gate,
        // so a `HashSet<&str>` is enough and gives O(1) contains
        // lookup (vs O(M) iter().any in the old shape).
        let mut reads_value: Vec<(&str, &str)> = Vec::with_capacity(8); // (target, field)
        let mut compares_age: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(8);

        visit(body, &mut |n| match n {
            Node::FieldAccess { target, field, .. } => {
                if let Node::Identifier { name: t, .. } = target.as_ref()
                    && !field.ends_with("_at")
                {
                    reads_value.push((t.as_str(), field.as_str()));
                }
            }
            Node::InfixExpression {
                operator,
                left,
                right,
                ..
            } if matches!(*operator, ">" | "<" | ">=" | "<=") => {
                for side in [left.as_ref(), right.as_ref()] {
                    if let Node::FieldAccess { target, field, .. } = side
                        && field.ends_with("_at")
                        && let Node::Identifier { name: t, .. } = target.as_ref()
                    {
                        compares_age.insert(t.as_str());
                    }
                }
            }
            _ => {}
        });

        for (target, field) in &reads_value {
            // If we read `target.field` and never compared `target.<anything>_at`,
            // that's an unguarded read. RES-2054: O(1) contains.
            if !compares_age.contains(target) {
                eprintln!(
                    "warning: in '{fname}', value '{target}.{field}' is read with \
                     no comparison on a sibling '*_at' timestamp — data may be stale"
                );
            }
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
    fn program_without_age_field_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
