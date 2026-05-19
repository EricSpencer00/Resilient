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
use crate::uniqueness_walk::{for_each_function, visit};

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
    // RES-1267 / RES-1917: the typechecker gates this call behind
    // `markers.any_let_name_with_suffix` and
    // `markers.any_param_name_with_suffix` with the same UNIT_SUFFIXES.
    // The previous `any_node` pre-scan was redundant — removed.
    for_each_function(program, |fname, params, body| {
        // RES-1748: pre-size to `params.len() + 8` — covers the
        // parameter seeds plus a small allowance for body-let
        // bindings with unit suffixes. Same shape as the pre-size
        // series across the per-fn passes.
        // RES-1950: values are `&'static str` from the static
        // UNIT_SUFFIXES table — no per-binding String allocation.
        //
        // RES-2058: keys now borrow `&str` directly from the AST
        // (parameter name in `params`, let-binding name in the body).
        // The map is dropped at the end of this closure call, so the
        // borrow lifetime is bounded by the closure's call lifetime —
        // both `params` and `body` outlive it. Eliminates a `name.clone()`
        // per inserted entry.
        let mut units: std::collections::HashMap<&str, &'static str> =
            std::collections::HashMap::with_capacity(params.len() + 8);
        for (_ty, name) in params {
            if let Some(u) = unit_of(name) {
                units.insert(name.as_str(), u);
            }
        }
        visit(body, &mut |n| {
            if let Node::LetStatement { name, value, .. } = n {
                if let Some(u) = unit_of(name) {
                    units.insert(name.as_str(), u);
                }
                check_expr(fname, name, value, &units);
            }
        });
    });
    Ok(())
}

// RES-1950: return the matched static slice directly instead of
// allocating a fresh String. `UNIT_SUFFIXES` entries are `&'static str`
// already, so wrapping them in `String::from` was pure overhead —
// paid once per parameter / let-binding whose name carried a unit
// suffix.
fn unit_of(name: &str) -> Option<&'static str> {
    UNIT_SUFFIXES.iter().find(|s| name.ends_with(*s)).copied()
}

fn check_expr(
    fname: &str,
    var: &str,
    expr: &Node,
    units: &std::collections::HashMap<&str, &'static str>,
) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        ..
    } = expr
    {
        if !matches!(*operator, "+" | "-") {
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

// RES-1950: returns `&'static str` so callers do a Copy instead of
// a String clone on lookup hit.
fn ident_unit(
    node: &Node,
    units: &std::collections::HashMap<&str, &'static str>,
) -> Option<&'static str> {
    if let Node::Identifier { name, .. } = node {
        return units.get(name.as_str()).copied().or_else(|| unit_of(name));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn no_unit_suffix_skips_check() {
        let src = "fn f(int x) -> int { return x + 1; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn same_unit_addition_returns_ok() {
        // V1 emits warnings but always returns Ok.
        let src = "fn f(int x_ms, int y_ms) -> int { return x_ms + y_ms; }\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "same-unit arithmetic must not error in V1"
        );
    }
}
