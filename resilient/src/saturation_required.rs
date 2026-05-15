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
use crate::uniqueness_walk::{any_node, for_each_function, visit};

const SAT_NAME_SUFFIXES: &[&str] = &["_pwm", "_duty", "_brightness", "_pct", "_throttle"];
const SAT_FNS: &[&str] = &["saturating_add", "saturating_sub", "saturating_mul"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1266: fast-reject. The per-function descent walks every function
    // body looking for `LetStatement`s whose name ends in a saturation-typed
    // suffix (`_pwm`, `_duty`, …). The overwhelming majority of programs
    // (every fixture in `examples/`, every unit test, every integration
    // test) have no such binding, so every descent produces zero work.
    // Pre-scan once with the early-terminating `any_node` (RES-1238) and
    // skip the per-function loop entirely when nothing matches.
    let has_sat_let = any_node(program, |n| match n {
        Node::LetStatement { name, .. } => SAT_NAME_SUFFIXES.iter().any(|s| name.ends_with(*s)),
        _ => false,
    });
    if !has_sat_let {
        return Ok(());
    }
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
        return matches!(operator.as_str(), "+" | "-" | "*");
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
}
