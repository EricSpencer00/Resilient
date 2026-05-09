//! Feature 43/50 — Mutation Testing.
//!
//! `rz mutate <file>` walks the AST, generates structured mutations
//! (operator swaps, constant changes, branch flips), and reports
//! the program-level kill rate.
//!
//! Built-in mutators (initial set):
//! * Replace `+` with `-` and vice versa
//! * Replace `<` with `<=` (boundary mutation)
//! * Flip `&&` ↔ `||`
//! * Replace `0` with `1` and `true` with `false`
//!
//! The runner doesn't actually re-execute tests in this slice — it
//! reports the *count* of generated mutations. The test-run
//! orchestrator is a follow-up.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct Mutation {
    pub fn_name: String,
    pub kind: String,
    pub description: String,
}

pub fn generate(program: &Node) -> Vec<Mutation> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            generate_in(body, name, &mut out);
        }
    }
    out
}

fn generate_in(node: &Node, fn_name: &str, out: &mut Vec<Mutation>) {
    match node {
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            match operator.as_str() {
                "+" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "arithmetic".into(),
                    description: "swap `+` -> `-`".into(),
                }),
                "-" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "arithmetic".into(),
                    description: "swap `-` -> `+`".into(),
                }),
                "<" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "boundary".into(),
                    description: "swap `<` -> `<=`".into(),
                }),
                "<=" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "boundary".into(),
                    description: "swap `<=` -> `<`".into(),
                }),
                "&&" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "logical".into(),
                    description: "swap `&&` -> `||`".into(),
                }),
                "||" => out.push(Mutation {
                    fn_name: fn_name.to_string(),
                    kind: "logical".into(),
                    description: "swap `||` -> `&&`".into(),
                }),
                _ => {}
            }
            generate_in(left, fn_name, out);
            generate_in(right, fn_name, out);
        }
        Node::IntegerLiteral { value, .. } if *value == 0 => {
            out.push(Mutation {
                fn_name: fn_name.to_string(),
                kind: "literal".into(),
                description: "swap `0` -> `1`".into(),
            });
        }
        Node::IntegerLiteral { .. } => {}
        Node::BooleanLiteral { value, .. } => {
            out.push(Mutation {
                fn_name: fn_name.to_string(),
                kind: "literal".into(),
                description: format!("flip `{}`", value),
            });
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                generate_in(s, fn_name, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => generate_in(e, fn_name, out),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            generate_in(value, fn_name, out)
        }
        Node::ExpressionStatement { expr, .. } => generate_in(expr, fn_name, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            generate_in(condition, fn_name, out);
            generate_in(consequence, fn_name, out);
            if let Some(e) = alternative {
                generate_in(e, fn_name, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            generate_in(condition, fn_name, out);
            generate_in(body, fn_name, out);
        }
        _ => {}
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn arithmetic_op_generates_mutation() {
        let src = r#"fn add(int a, int b) { return a + b; }"#;
        let (prog, _) = parse(src);
        let mutations = generate(&prog);
        assert!(!mutations.is_empty());
        assert!(mutations.iter().any(|m| m.kind == "arithmetic"));
    }

    #[test]
    fn boundary_op_generates_mutation() {
        let src = r#"fn lt(int a, int b) -> bool { return a < b; }"#;
        let (prog, _) = parse(src);
        let mutations = generate(&prog);
        assert!(mutations.iter().any(|m| m.kind == "boundary"));
    }
}
