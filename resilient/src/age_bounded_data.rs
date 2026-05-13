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
use crate::uniqueness_walk::{any_node, for_each_function, visit};

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1271: fast-reject. The whole point of this pass is to find
    // `target.field` reads that aren't gated by a `target.*_at`
    // comparison. If the program has no `*_at` field anywhere, the
    // pattern doesn't apply — every read would warn indiscriminately
    // and the warnings are meaningless for programs that don't use
    // the age-bounded convention. Pre-scan for any `FieldAccess`
    // whose field name ends in `_at`; if none, skip the entire pass.
    let has_age_field = any_node(program, |n| match n {
        Node::FieldAccess { field, .. } => field.ends_with("_at"),
        _ => false,
    });
    if !has_age_field {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        // RES-1750: pre-size per-fn collections to 8 — typical fn
        // body has a handful of field reads / age comparisons.
        let mut reads_value: Vec<(String, String)> = Vec::with_capacity(8); // (target, field)
        let mut compares_age: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::with_capacity(8);

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
