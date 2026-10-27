//! Type correspondence for the integer-encoded Lean expression evaluator.

use super::{LeanExpr, LowerError};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Type {
    Int,
    Bool,
}

fn unsupported(message: impl Into<String>) -> LowerError {
    LowerError::UnsupportedNode(message.into())
}

fn parse_type(name: &str) -> Result<Type, LowerError> {
    match name {
        "int" => Ok(Type::Int),
        "bool" => Ok(Type::Bool),
        _ => Err(unsupported(format!(
            "unsupported Lean signature type `{name}`"
        ))),
    }
}

pub(super) fn validate(
    expr: &LeanExpr,
    parameters: &[(String, String)],
    result: Option<&str>,
) -> Result<(), LowerError> {
    let mut environment = HashMap::new();
    for (ty, name) in parameters {
        if environment.insert(name.as_str(), parse_type(ty)?).is_some() {
            return Err(unsupported(format!("duplicate parameter `{name}`")));
        }
    }
    let actual = infer(expr, &environment)?;
    if let Some(result) = result {
        if parse_type(result)? != actual {
            return Err(unsupported(
                "return expression does not match declared type",
            ));
        }
    }
    Ok(())
}

fn infer(expr: &LeanExpr, environment: &HashMap<&str, Type>) -> Result<Type, LowerError> {
    use LeanExpr::*;
    match expr {
        Int(_) => Ok(Type::Int),
        Bool(_) => Ok(Type::Bool),
        Var(name) => environment
            .get(name.as_str())
            .copied()
            .ok_or_else(|| unsupported(format!("unbound identifier `{name}`"))),
        Neg(inner) | Not(inner) => {
            let expected = if matches!(expr, Neg(_)) {
                Type::Int
            } else {
                Type::Bool
            };
            if infer(inner, environment)? != expected {
                return Err(unsupported("invalid unary operand type"));
            }
            Ok(expected)
        }
        Ite(condition, yes, no) => {
            let branch = infer(yes, environment)?;
            if infer(condition, environment)? != Type::Bool || infer(no, environment)? != branch {
                return Err(unsupported("invalid conditional expression types"));
            }
            Ok(branch)
        }
        Add(left, right)
        | Sub(left, right)
        | Mul(left, right)
        | Div(left, right)
        | Mod(left, right)
        | Eq(left, right)
        | Lt(left, right)
        | Le(left, right) => {
            let left = infer(left, environment)?;
            let right = infer(right, environment)?;
            if left != right || (!matches!(expr, Eq(..)) && left != Type::Int) {
                return Err(unsupported("invalid binary operand types"));
            }
            Ok(if matches!(expr, Eq(..) | Lt(..) | Le(..)) {
                Type::Bool
            } else {
                Type::Int
            })
        }
    }
}

pub(super) fn parameter(index: usize, ty: &str) -> String {
    let name = format!("lean_arg_{index}");
    if ty == "bool" {
        format!("(if {name} then (1 : Int) else 0)")
    } else {
        name
    }
}

pub(super) fn native(expr: &LeanExpr, parameters: &[(String, String)]) -> String {
    use LeanExpr::*;
    let render = |expr| native(expr, parameters);
    let boolean = |condition: String| format!("(if {condition} then (1 : Int) else 0)");
    match expr {
        Int(value) => format!("({value} : Int)"),
        Bool(value) => if *value { "(1 : Int)" } else { "(0 : Int)" }.into(),
        Var(name) => parameters
            .iter()
            .enumerate()
            .find(|(_, (_, candidate))| candidate == name)
            .map(|(index, (ty, _))| parameter(index, ty))
            .unwrap_or_else(|| name.clone()),
        Add(l, r) => format!("({} + {})", render(l), render(r)),
        Sub(l, r) => format!("({} - {})", render(l), render(r)),
        Mul(l, r) => format!("({} * {})", render(l), render(r)),
        Div(l, r) => format!("({} / {})", render(l), render(r)),
        Mod(l, r) => format!("({} % {})", render(l), render(r)),
        Eq(l, r) => boolean(format!("{} = {}", render(l), render(r))),
        Lt(l, r) => boolean(format!("{} < {}", render(l), render(r))),
        Le(l, r) => boolean(format!("{} ≤ {}", render(l), render(r))),
        Neg(inner) => format!("(-{})", render(inner)),
        Not(inner) => boolean(format!("{} = 0", render(inner))),
        Ite(c, t, e) => format!(
            "(if {} = 0 then {} else {})",
            render(c),
            render(e),
            render(t)
        ),
    }
}

pub(super) fn conditions(expr: &LeanExpr, parameters: &[(String, String)]) -> Vec<String> {
    use LeanExpr::*;
    let mut result = Vec::new();
    match expr {
        Int(_) | Bool(_) | Var(_) => {}
        Neg(inner) | Not(inner) => result.extend(conditions(inner, parameters)),
        Ite(c, t, e) => {
            for inner in [c, t, e] {
                result.extend(conditions(inner, parameters));
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
            result.extend(conditions(l, parameters));
            result.extend(conditions(r, parameters));
            let operator = match expr {
                Eq(..) => Some("="),
                Lt(..) => Some("<"),
                Le(..) => Some("≤"),
                _ => None,
            };
            if let Some(operator) = operator {
                result.push(format!(
                    "{} {operator} {}",
                    native(l, parameters),
                    native(r, parameters)
                ));
            }
        }
    }
    result.sort();
    result.dedup();
    result
}
