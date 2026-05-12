//! Ralph-Loop Uniqueness #12 — numeric-unit mixing detection by suffix.
//!
//! The Mars Climate Orbiter loss in 1999 came from mixing pound-seconds
//! with newton-seconds. F# has Units of Measure (great!) but they're
//! library-defined and only checked when you opt in. Ada has dimensions
//! via custom types. Rust requires `uom` crate.
//!
//! Resilient checks unit consistency *by name*. If a `let` binds a
//! variable suffixed `_ms`, `_s`, `_us`, `_ns`, `_m`, `_cm`, `_mm`,
//! `_km`, `_kg`, `_g`, `_n`, `_v`, `_a`, or any of the SI suffixes
//! we recognize, then any binary `+`/`-` involving it must operate on
//! a same-suffix expression. Mixing `_ms` with `_s` in `+`/`-` warns.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function, visit};

const UNIT_SUFFIXES: &[&str] = &[
    "_ms", "_s", "_us", "_ns", // time
    "_m", "_cm", "_mm", "_km", // length
    "_kg", "_g", // mass
    "_n", // force
    "_v", "_mv", // voltage
    "_a", "_ma", // current
    "_hz", "_khz", "_mhz", // frequency
];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1267: fast-reject. The per-function body walk only does
    // anything when it finds a `LetStatement` whose name matches a
    // unit suffix (`_ms` / `_s` / `_m` / `_kg` / …). Function params
    // can also seed the units map. For programs without any
    // unit-suffixed binding (the overwhelming majority of `cargo
    // test` inputs and the entire `examples/` tree), every
    // per-function visit produces nothing.
    //
    // Pre-scan the program once via `any_node` (RES-1238 made this
    // early-terminating) for any `LetStatement` or `Function` param
    // whose name matches a unit suffix. If none, return `Ok(())`
    // immediately. The pre-scan also examines `Node::Function`
    // params because the original closure seeds the units map from
    // both sources.
    let has_unit_binding = any_node(program, |n| match n {
        Node::LetStatement { name, .. } => unit_of(name).is_some(),
        Node::Function { parameters, .. } => parameters.iter().any(|(_ty, p)| unit_of(p).is_some()),
        _ => false,
    });
    if !has_unit_binding {
        return Ok(());
    }
    for_each_function(program, |fname, params, body| {
        let mut units: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for (_ty, name) in params {
            if let Some(u) = unit_of(name) {
                units.insert(name.clone(), u);
            }
        }
        visit(body, &mut |n| {
            if let Node::LetStatement { name, value, .. } = n {
                if let Some(u) = unit_of(name) {
                    units.insert(name.clone(), u);
                }
                check_expr(fname, name, value, &units);
            }
        });
    });
    Ok(())
}

fn unit_of(name: &str) -> Option<String> {
    UNIT_SUFFIXES
        .iter()
        .find(|s| name.ends_with(*s))
        .map(|s| (*s).to_string())
}

fn check_expr(
    fname: &str,
    var: &str,
    expr: &Node,
    units: &std::collections::HashMap<String, String>,
) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        ..
    } = expr
    {
        if !matches!(operator.as_str(), "+" | "-") {
            return;
        }
        let lu = ident_unit(left, units);
        let ru = ident_unit(right, units);
        if let (Some(l), Some(r)) = (lu, ru) {
            if l != r {
                eprintln!(
                    "warning: in '{fname}', let '{var}' adds/subtracts mixed units \
                     '{l}' and '{r}' — likely a unit conversion bug"
                );
            }
        }
    }
}

fn ident_unit(node: &Node, units: &std::collections::HashMap<String, String>) -> Option<String> {
    if let Node::Identifier { name, .. } = node {
        return units.get(name).cloned().or_else(|| unit_of(name));
    }
    None
}
