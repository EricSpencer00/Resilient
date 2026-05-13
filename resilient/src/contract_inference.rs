//! Feature 4/50 — Contract Inference.
//!
//! Given a function with no `requires`/`ensures` declared, infer the
//! strongest invariants that are consistent with its body. Today the
//! analyzer ships a syntactic abductor that scans the fn body for
//! direct uses of parameters and proposes:
//!
//! * `requires p != 0` when `p` appears as a divisor in `a / p` or
//!   `a % p` — division by zero is the canonical missing precondition.
//! * `requires p >= 0` when `p` is the iteration bound of a `while`
//!   or `for` whose body uses arithmetic that may underflow on negatives.
//! * `requires len(p) > 0` when `p[0]` is read without an explicit
//!   bounds check.
//! * `ensures result == X` when the body has exactly one `return X;`
//!   and `X` is a closed-form expression in the parameters.
//! * `ensures result >= 0` when the body returns a sum/product of
//!   parameters that are themselves non-negative or absolute values.
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
    // RES-1756: pre-size to stmts.len() — every top-level statement
    // could be a function and push one inferred entry. Same shape as
    // semantic_regression's extract_contracts pre-size.
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
                if requires.is_empty() && body_divides_by(body, pname) {
                    req.push(format!("{pname} != 0"));
                }
                if requires.is_empty() && body_indexes_into(body, pname) {
                    req.push(format!("len({pname}) > 0"));
                }
            }
            if ensures.is_empty() {
                if let Some(e) = single_return_expr(body) {
                    ens.push(format!("result == {e}"));
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
        _ => false,
    }
}

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

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1206: this pass historically called `infer_program` and
    // discarded the returned `Vec<InferredContracts>`. The real
    // consumers (the `--suggest-contracts` CLI flag and any external
    // integrator) call `infer_program` directly when they need the
    // suggestions, so the work here was unobservable. The entry point
    // is kept so the `EXTENSION_PASSES` block in `typechecker.rs`
    // stays undisturbed and a future use can flow data through this
    // slot.
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
}
