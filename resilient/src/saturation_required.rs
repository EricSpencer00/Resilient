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
