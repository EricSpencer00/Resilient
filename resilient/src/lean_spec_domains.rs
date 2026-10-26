//! Definedness conditions for the partial expression evaluator.

use super::{LeanExpr, typed};

pub(super) fn domain(expr: &LeanExpr, parameters: &[(String, String)]) -> String {
    use LeanExpr::*;
    let mut conditions = Vec::new();
    match expr {
        Int(_) | Bool(_) | Var(_) => {}
        Neg(inner) | Not(inner) => conditions.push(domain(inner, parameters)),
        Ite(condition, yes, no) => {
            conditions.push(domain(condition, parameters));
            conditions.push(format!(
                "(if {} = 0 then {} else {})",
                typed::native(condition, parameters),
                domain(no, parameters),
                domain(yes, parameters)
            ));
        }
        Add(l, r)
        | Sub(l, r)
        | Mul(l, r)
        | Div(l, r)
        | Mod(l, r)
        | Eq(l, r)
        | Lt(l, r)
        | Le(l, r) => {
            conditions.push(domain(l, parameters));
            conditions.push(domain(r, parameters));
            if matches!(expr, Div(..) | Mod(..)) {
                conditions.push(format!("({} ≠ 0)", typed::native(r, parameters)));
            }
        }
    }
    conditions.retain(|condition| condition != "True");
    match conditions.as_slice() {
        [] => "True".into(),
        [only] => only.clone(),
        _ => format!("({})", conditions.join(" ∧ ")),
    }
}

pub(super) fn result(expr: &LeanExpr, parameters: &[(String, String)]) -> String {
    let domain = domain(expr, parameters);
    let value = typed::native(expr, parameters);
    if domain == "True" {
        format!("some {value}")
    } else {
        format!("(if {domain} then some {value} else none)")
    }
}

pub(super) fn zero_cases(expr: &LeanExpr, parameters: &[(String, String)]) -> Vec<String> {
    use LeanExpr::*;
    let mut result = Vec::new();
    match expr {
        Int(_) | Bool(_) | Var(_) => {}
        Neg(inner) | Not(inner) => result.extend(zero_cases(inner, parameters)),
        Ite(c, t, e) => {
            for inner in [c, t, e] {
                result.extend(zero_cases(inner, parameters));
            }
        }
        Add(l, r)
        | Sub(l, r)
        | Mul(l, r)
        | Div(l, r)
        | Mod(l, r)
        | Eq(l, r)
        | Lt(l, r)
        | Le(l, r) => {
            result.extend(zero_cases(l, parameters));
            result.extend(zero_cases(r, parameters));
            if matches!(expr, Div(..) | Mod(..)) {
                result.push(format!("{} = 0", typed::native(r, parameters)));
            }
        }
    }
    result.sort();
    result.dedup();
    result
}
