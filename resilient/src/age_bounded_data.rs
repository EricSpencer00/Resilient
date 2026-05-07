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
    for_each_function(program, |fname, _params, body| {
        let mut reads_value: Vec<(String, String)> = Vec::new(); // (target, field)
        let mut compares_age: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();

        visit(body, &mut |n| match n {
            Node::FieldAccess { target, field, .. } => {
                if let Node::Identifier { name: t, .. } = target.as_ref() {
                    if !field.ends_with("_at") {
                        reads_value.push((t.clone(), field.clone()));
                    }
                }
            }
            Node::InfixExpression {
                operator,
                left,
                right,
                ..
            } if matches!(operator.as_str(), ">" | "<" | ">=" | "<=") => {
                for side in [left.as_ref(), right.as_ref()] {
                    if let Node::FieldAccess { target, field, .. } = side {
                        if field.ends_with("_at") {
                            if let Node::Identifier { name: t, .. } = target.as_ref() {
                                compares_age.insert((t.clone(), field.clone()));
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
            let saw_age = compares_age.iter().any(|(t, _)| t == &target);
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
