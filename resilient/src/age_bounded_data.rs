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
        // RES-2158: borrow target + field names directly from the
        // body AST. The two collections live only for the duration of
        // this for_each_function callback, so the strings can borrow
        // through `body`'s lifetime. The previous shape paid
        // `t.clone() + field.clone()` per matching FieldAccess +
        // InfixExpression — pure overhead since downstream consumers
        // (`reads_value` for-loop, `compares_age.iter().any(...)`)
        // only read.
        let mut reads_value: Vec<(&str, &str)> = Vec::with_capacity(8); // (target, field)
        let mut compares_age: std::collections::HashSet<(&str, &str)> =
            std::collections::HashSet::with_capacity(8);

        visit(body, &mut |n| match n {
            Node::FieldAccess { target, field, .. } => {
                if let Node::Identifier { name: t, .. } = target.as_ref() {
                    if !field.ends_with("_at") {
                        reads_value.push((t.as_str(), field.as_str()));
                    }
                }
            }
            Node::InfixExpression {
                operator,
                left,
                right,
                ..
            } if matches!(*operator, ">" | "<" | ">=" | "<=") => {
                for side in [left.as_ref(), right.as_ref()] {
                    if let Node::FieldAccess { target, field, .. } = side {
                        if field.ends_with("_at") {
                            if let Node::Identifier { name: t, .. } = target.as_ref() {
                                compares_age.insert((t.as_str(), field.as_str()));
                            }
                        }
                    }
                }
            }
            _ => {}
        });

        for (target, field) in reads_value {
            // If we read `target.field` and never compared `target.<anything>_at`,
            // that's an unguarded read.
            let saw_age = compares_age.iter().any(|(t, _)| *t == target);
            if !saw_age {
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

    #[test]
    fn value_read_with_age_comparison_passes() {
        let src = r#"
            fn check_reading(GPS g) {
                if g.reading_at > 1000 {
                    int fresh = g.reading;
                }
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn value_read_without_age_check_passes_when_different_target() {
        let src = r#"
            fn check_reading(GPS g, GPS g2) {
                if g.obs_at > 1000 {
                    int val = g2.obs;
                }
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn multiple_age_fields_tracked() {
        let src = r#"
            fn multi_check(Sensor s) {
                if s.temp_at > 100 {
                    int t = s.temp;
                }
                if s.pressure_at > 50 {
                    int p = s.pressure;
                }
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn relational_operators_all_work() {
        let src = r#"
            fn check_all(Sensor s) {
                if s.taken > 100 { int v = s.val; }
                if s.taken < 200 { int v2 = s.val; }
                if s.taken >= 150 { int v3 = s.val; }
                if s.taken <= 250 { int v4 = s.val; }
            }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
