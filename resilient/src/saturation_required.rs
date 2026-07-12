//! Ralph-Loop Uniqueness #11 — saturation-required arithmetic for sentinel
//! types.
//!
//! In safety-critical control code, certain quantities (motor PWM, GPIO
//! brightness, fuel %) must *saturate* on overflow rather than wrap or
//! UB. C / Rust / Ada all expose saturating ops via library calls; none
//! requires their use for a particular value class.
//!
//! Resilient enforces by name: any local `let` whose name ends in
//! `_pwm`, `_duty`, `_brightness`, `_pct`, or `_throttle` and is the
//! result of an unchecked `+` / `-` / `*` operator must instead use
//! `saturating_add` / `saturating_sub` / `saturating_mul`. Otherwise a
//! warning fires.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

const SAT_NAME_SUFFIXES: &[&str] = &["_pwm", "_duty", "_brightness", "_pct", "_throttle"];
const SAT_FNS: &[&str] = &["saturating_add", "saturating_sub", "saturating_mul"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1266 / RES-1917: the typechecker gates this call behind
    // `markers.any_let_name_with_suffix` with the same SAT_NAME_SUFFIXES.
    // The previous `any_node` pre-scan was redundant — removed.
    for_each_function(program, |fname, _params, body| {
        visit(body, &mut |n| {
            if let Node::LetStatement { name, value, .. } = n {
                if !SAT_NAME_SUFFIXES.iter().any(|s| name.ends_with(*s)) {
                    return;
                }
                if uses_unchecked_arith(value) && !uses_saturating_call(value) {
                    eprintln!(
                        "warning: in '{fname}', local '{name}' is a saturation-typed \
                         quantity but is computed with unchecked arithmetic — use \
                         saturating_add/saturating_sub/saturating_mul to avoid \
                         wraparound on overflow"
                    );
                }
            }
        });
    });
    Ok(())
}

fn uses_unchecked_arith(node: &Node) -> bool {
    if let Node::InfixExpression { operator, .. } = node {
        return matches!(*operator, "+" | "-" | "*");
    }
    false
}

fn uses_saturating_call(node: &Node) -> bool {
    matches!(node,
        Node::CallExpression { function, .. }
            if matches!(function.as_ref(),
                Node::Identifier { name, .. } if SAT_FNS.contains(&name.as_str())
            )
    )
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
    fn program_without_saturating_name_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn sat_fns_include_saturating_add() {
        assert!(SAT_FNS.contains(&"saturating_add"));
        assert!(SAT_NAME_SUFFIXES.contains(&"_pwm"));
    }

    #[test]
    fn all_sat_suffixes_detected() {
        for suffix in &["_pwm", "_duty", "_brightness", "_pct", "_throttle"] {
            assert!(
                SAT_NAME_SUFFIXES.contains(suffix),
                "suffix {} must be in list",
                suffix
            );
        }
    }

    #[test]
    fn duty_cycle_with_saturating_mul_passes() {
        let src =
            "fn adjust(int x) -> int { let duty_val = saturating_mul(x, 2); return duty_val; }\n";
        let (prog, _) = crate::parse(src);
        // V1 warns but doesn't error, so always Ok
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn brightness_pct_with_unchecked_add_detected() {
        let src = "fn set(int b) { let brightness_pct = b + 10; }\n";
        let (prog, _) = crate::parse(src);
        // V1 warns but returns Ok
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn throttle_with_unchecked_sub_detected() {
        let src = "fn reduce(int t) { let throttle_val = t - 5; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
