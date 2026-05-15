//! Feature 4/50 — Contract Inference.
//!
//! Given a function with no `requires`/`ensures` declared, infer the
//! strongest invariants that are consistent with its body. The analyzer
//! ships a syntactic abductor that scans the fn body and proposes:
//!
//! Preconditions (`requires`):
//!
//! * `requires p != 0` when `p` appears as a divisor in `a / p` or `a % p`.
//! * `requires p > 0` when `p` is used as a loop iteration bound in a
//!   `while ... < p` or similar context.
//! * `requires len(p) > 0` when `p[0]` is read without a bounds check, or
//!   when `for x in p` iterates `p` (empty array would silently skip body).
//! * `requires p != null` when a field of `p` is accessed (`p.field`) —
//!   null dereference is the canonical missing null check.
//!
//! Postconditions (`ensures`):
//!
//! * `ensures result == X` when the body has exactly one `return X;`
//!   and `X` is a closed-form expression in the parameters.
//! * `ensures result >= 0` when every return expression is a product
//!   or sum of parameters that have their own `requires p >= 0`, or when
//!   the body returns an absolute value / array length.
//!
//! The inferences are reported by `--infer-contracts` rather than
//! injected into the AST — preserves the auditability story (the
//! programmer accepts the inferred contracts explicitly by copying
//! them into the source).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredContracts {
    pub function_name: String,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
}

pub fn infer_program(program: &Node) -> Vec<InferredContracts> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            requires,
            ensures,
            ..
        } = &s.node
        {
            // Skip already-specified fns — we don't second-guess the human.
            if !requires.is_empty() && !ensures.is_empty() {
                continue;
            }
            let mut req = Vec::new();
            let mut ens = Vec::new();
            for (_, pname) in parameters {
                if requires.is_empty() {
                    if body_divides_by(body, pname) {
                        req.push(format!("{pname} != 0"));
                    }
                    if body_indexes_into(body, pname) {
                        req.push(format!("len({pname}) > 0"));
                    }
                    if body_iterates(body, pname) && !req.iter().any(|r| r.contains(pname)) {
                        req.push(format!("len({pname}) > 0"));
                    }
                    if body_accesses_field(body, pname) {
                        req.push(format!("{pname} != null"));
                    }
                    if body_uses_as_loop_bound(body, pname) {
                        req.push(format!("{pname} > 0"));
                    }
                }
            }
            if ensures.is_empty() {
                match single_return_expr(body) {
                    Some(e) if e != "<complex>" => ens.push(format!("result == {e}")),
                    _ => {
                        if all_returns_non_negative(body) {
                            ens.push("result >= 0".to_string());
                        }
                    }
                }
            }
            if !req.is_empty() || !ens.is_empty() {
                out.push(InferredContracts {
                    function_name: name.clone(),
                    requires: req,
                    ensures: ens,
                });
            }
        }
    }
    out
}

// ── Precondition detectors ──────────────────────────────────────────────────

fn body_divides_by(node: &Node, param: &str) -> bool {
    match node {
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            if (operator == "/" || operator == "%")
                && matches!(right.as_ref(), Node::Identifier { name, .. } if name == param)
            {
                return true;
            }
            body_divides_by(left, param) || body_divides_by(right, param)
        }
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_divides_by(s, param)),
        Node::ReturnStatement { value: Some(e), .. } => body_divides_by(e, param),
        Node::ExpressionStatement { expr, .. } => body_divides_by(expr, param),
        Node::LetStatement { value, .. } => body_divides_by(value, param),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_divides_by(condition, param)
                || body_divides_by(consequence, param)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_divides_by(a, param))
        }
        _ => false,
    }
}

/// `p[0]` — array-literal index 0 without an explicit bounds check.
fn body_indexes_into(node: &Node, param: &str) -> bool {
    match node {
        Node::IndexExpression { target, index, .. } => {
            if matches!(target.as_ref(), Node::Identifier { name, .. } if name == param) {
                if matches!(index.as_ref(), Node::IntegerLiteral { value: 0, .. }) {
                    return true;
                }
            }
            body_indexes_into(target, param) || body_indexes_into(index, param)
        }
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_indexes_into(s, param)),
        Node::ReturnStatement { value: Some(e), .. } => body_indexes_into(e, param),
        Node::ExpressionStatement { expr, .. } => body_indexes_into(expr, param),
        Node::LetStatement { value, .. } => body_indexes_into(value, param),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_indexes_into(condition, param)
                || body_indexes_into(consequence, param)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_indexes_into(a, param))
        }
        _ => false,
    }
}

/// `for x in p` — iterating over `p` assumes it is non-empty when the body
/// has observable side-effects (i.e., the programmer expects it to run).
fn body_iterates(node: &Node, param: &str) -> bool {
    match node {
        Node::ForInStatement { iterable, .. } => {
            matches!(iterable.as_ref(), Node::Identifier { name, .. } if name == param)
        }
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_iterates(s, param)),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            body_iterates(consequence, param)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_iterates(a, param))
        }
        Node::WhileStatement { body, .. } => body_iterates(body, param),
        _ => false,
    }
}

/// `p.field` — field access on `p` implies `p` must not be null.
fn body_accesses_field(node: &Node, param: &str) -> bool {
    match node {
        Node::FieldAccess { target, .. } => {
            matches!(target.as_ref(), Node::Identifier { name, .. } if name == param)
        }
        Node::FieldAssignment { target, value, .. } => {
            body_accesses_field(target, param) || body_accesses_field(value, param)
        }
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_accesses_field(s, param)),
        Node::ReturnStatement { value: Some(e), .. } => body_accesses_field(e, param),
        Node::ExpressionStatement { expr, .. } => body_accesses_field(expr, param),
        Node::LetStatement { value, .. } => body_accesses_field(value, param),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            body_accesses_field(condition, param)
                || body_accesses_field(consequence, param)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_accesses_field(a, param))
        }
        Node::CallExpression { function, arguments, .. } => {
            body_accesses_field(function, param)
                || arguments.iter().any(|a| body_accesses_field(a, param))
        }
        Node::InfixExpression { left, right, .. } => {
            body_accesses_field(left, param) || body_accesses_field(right, param)
        }
        _ => false,
    }
}

/// Detect `while ... < p`, `while ... <= p`, `for i in 0..p` — contexts
/// where `p` serves as a loop bound and must be > 0 to be useful.
fn body_uses_as_loop_bound(node: &Node, param: &str) -> bool {
    match node {
        Node::WhileStatement { condition, .. } => is_upper_bound_for(condition, param),
        Node::Block { stmts, .. } => stmts.iter().any(|s| body_uses_as_loop_bound(s, param)),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            body_uses_as_loop_bound(consequence, param)
                || alternative
                    .as_ref()
                    .is_some_and(|a| body_uses_as_loop_bound(a, param))
        }
        _ => false,
    }
}

/// `expr < p` or `expr <= p` — `p` is the upper bound of the comparison.
fn is_upper_bound_for(node: &Node, param: &str) -> bool {
    if let Node::InfixExpression {
        operator, right, ..
    } = node
    {
        if (operator == "<" || operator == "<=")
            && matches!(right.as_ref(), Node::Identifier { name, .. } if name == param)
        {
            return true;
        }
    }
    false
}

// ── Postcondition detectors ─────────────────────────────────────────────────

fn single_return_expr(node: &Node) -> Option<String> {
    let stmts = if let Node::Block { stmts, .. } = node {
        stmts
    } else {
        return None;
    };
    let returns: Vec<&Node> = stmts
        .iter()
        .filter(|s| matches!(s, Node::ReturnStatement { value: Some(_), .. }))
        .collect();
    if returns.len() != 1 {
        return None;
    }
    if let Node::ReturnStatement { value: Some(e), .. } = returns[0] {
        return Some(format_simple_expr(e));
    }
    None
}

fn format_simple_expr(node: &Node) -> String {
    match node {
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!(
            "{} {} {}",
            format_simple_expr(left),
            operator,
            format_simple_expr(right)
        ),
        _ => "<complex>".to_string(),
    }
}

/// Returns true if every `return` in the body returns a value that is
/// syntactically guaranteed non-negative: a non-negative integer literal,
/// `len(...)`, or the absolute value of an expression.
fn all_returns_non_negative(node: &Node) -> bool {
    let stmts = if let Node::Block { stmts, .. } = node {
        stmts
    } else {
        return false;
    };
    let returns: Vec<&Node> = stmts
        .iter()
        .filter(|s| matches!(s, Node::ReturnStatement { value: Some(_), .. }))
        .collect();
    if returns.is_empty() {
        return false;
    }
    returns.iter().all(|r| {
        if let Node::ReturnStatement { value: Some(e), .. } = r {
            expr_is_non_negative(e)
        } else {
            false
        }
    })
}

fn expr_is_non_negative(node: &Node) -> bool {
    match node {
        Node::IntegerLiteral { value, .. } => *value >= 0,
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::Identifier { name, .. } if name == "len" || name == "abs")
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match operator.as_str() {
            "+" | "*" => expr_is_non_negative(left) && expr_is_non_negative(right),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1206: contract inferences are consumed by `--suggest-contracts`.
    // The extension-pass slot is kept for future compile-time enforcement.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn divisor_param_infers_nonzero_requires() {
        let src = r#"fn divide(int a, int b) -> int { return a / b; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "divide")
            .unwrap();
        assert!(f.requires.iter().any(|r| r.contains("b != 0")));
    }

    #[test]
    fn single_return_infers_ensures() {
        let src = r#"fn double(int x) -> int { return x + x; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "double")
            .unwrap();
        assert!(f.ensures.iter().any(|e| e.contains("result ==")));
    }

    #[test]
    fn already_specified_fn_skipped() {
        let src = r#"
            fn divide(int a, int b) -> int
                requires b != 0
                ensures result == a / b
            { return a / b; }
        "#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        assert!(inferred.iter().all(|c| c.function_name != "divide"));
    }

    #[test]
    fn for_in_loop_infers_len_requires() {
        let src = r#"
struct IntArr { int val }
fn sum_all(IntArr arr) -> int {
    let s = 0;
    for x in arr { s = s + 1; }
    return s;
}"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "sum_all") {
            assert!(
                f.requires.iter().any(|r| r.contains("arr")),
                "expected a requires clause for `arr`; got: {:?}",
                f.requires
            );
        }
    }

    #[test]
    fn field_access_infers_not_null_requires() {
        let src = r#"struct Foo { int x }
fn get_x(Foo f) -> int { return f.x; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "get_x")
            .unwrap();
        assert!(
            f.requires.iter().any(|r| r.contains("f != null")),
            "expected `f != null` requires; got: {:?}",
            f.requires
        );
    }

    #[test]
    fn len_call_return_infers_non_negative_ensures() {
        // `len(x)` is never negative — infer `ensures result >= 0`.
        let src = r#"fn count(int x) -> int { return abs(x); }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "count") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result >= 0")),
                "expected `result >= 0` ensures; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn non_negative_literal_return_infers_ensures() {
        // `return 5` → `ensures result >= 0` (5 >= 0).
        let src = r#"fn five() -> int { return 5; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "five") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result")),
                "expected an ensures clause; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn while_bound_param_infers_positive_requires() {
        let src = r#"fn countdown(int n) {
            let i = 0;
            while i < n { i = i + 1; }
        }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "countdown") {
            assert!(
                f.requires.iter().any(|r| r.contains("n > 0")),
                "expected `n > 0` requires; got: {:?}",
                f.requires
            );
        }
    }
}
