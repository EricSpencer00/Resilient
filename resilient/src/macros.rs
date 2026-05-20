//! Feature 39/50 — Macros (Compile-Time Substitution).
//!
//! `#[macro(pattern = "...", expansion = "...")]` declares a simple
//! syntactic macro: when the parser sees a call to the macro's name,
//! it substitutes the expansion template (with `$arg` placeholders
//! filled in from the call site).
//!
//! This is a textual macro system (not hygienic), suitable for
//! `assert_eq!`, `format!`, and small DSLs. Hygiene + procedural
//! macros are downstream tickets.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone)]
pub struct MacroDef {
    pub name: String,
    pub pattern: String,
    pub expansion: String,
}

static MACROS: LazyLock<RwLock<HashMap<String, MacroDef>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<MacroDef> {
    let attrs = crate::feature_attrs::find_kind("macro");
    // RES-1764: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut pattern = String::new();
        let mut expansion = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "pattern" => pattern = v.to_string(),
                    "expansion" => expansion = v.to_string(),
                    _ => {}
                }
            }
        }
        out.push(MacroDef {
            name: item,
            pattern,
            expansion,
        });
    }
    out
}

pub fn install(macros: Vec<MacroDef>) {
    if let Ok(mut g) = MACROS.write() {
        g.clear();
        for m in macros {
            g.insert(m.name.clone(), m);
        }
    }
}

pub fn expand(name: &str, args: &[String]) -> Option<String> {
    let g = MACROS.read().ok()?;
    let def = g.get(name)?;
    let mut out = def.expansion.clone();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("${}", i + 1), a);
    }
    Some(out)
}

/// AST-level macro expansion pass.
///
/// Called during the lowering pipeline (after newtypes::lower_program)
/// so that expanded forms participate in typechecking and evaluation.
///
/// For every `CallExpression` whose callee is a registered `#[macro(...)]`
/// name:
/// 1. Serialize each argument back to a source string.
/// 2. Substitute into the expansion template.
/// 3. Re-parse the result as a single expression via
///    `crate::parse_single_expression`.
/// 4. Replace the call node in place.
///
/// Expansions that fail to parse (e.g. multi-statement bodies) are left
/// unexpanded — the typechecker will emit an "unknown function" error, which
/// is more useful than a silent no-op.
pub fn lower_program(program: &mut Node) {
    let macros_snapshot: Vec<(String, MacroDef)> = {
        let Ok(g) = MACROS.read() else { return };
        if g.is_empty() {
            // Fast path: no macros installed — also try feature_attrs
            // in case check() hasn't run yet (e.g. during lowering before
            // the EXTENSION_PASSES typecheck phase).
            drop(g);
            let defs = collect();
            if defs.is_empty() {
                return;
            }
            install(defs.clone());
            defs.into_iter().map(|d| (d.name.clone(), d)).collect()
        } else {
            g.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        }
    };
    if macros_snapshot.is_empty() {
        return;
    }
    let macro_names: std::collections::HashSet<String> =
        macros_snapshot.iter().map(|(k, _)| k.clone()).collect();
    lower_node(program, &macro_names);
}

fn lower_node(node: &mut Node, macro_names: &std::collections::HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            // Recurse into arguments first (inside-out expansion).
            for arg in arguments.iter_mut() {
                lower_node(arg, macro_names);
            }
            if let Node::Identifier { name, .. } = function.as_ref() {
                if macro_names.contains(name) {
                    let arg_strs: Vec<String> = arguments.iter().map(node_to_source).collect();
                    if let Some(expanded) = expand(name, &arg_strs) {
                        if let Some(expanded_node) = crate::parse_single_expression(&expanded) {
                            *node = expanded_node;
                        }
                    }
                }
            }
        }
        Node::Program(items) => {
            for s in items.iter_mut() {
                lower_node(&mut s.node, macro_names);
            }
        }
        Node::Function { body, .. } => lower_node(body, macro_names),
        Node::Block { stmts, .. } => {
            for s in stmts.iter_mut() {
                lower_node(s, macro_names);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => lower_node(value, macro_names),
        Node::ReturnStatement { value: Some(v), .. } => lower_node(v, macro_names),
        Node::ExpressionStatement { expr, .. } => lower_node(expr, macro_names),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            lower_node(condition, macro_names);
            lower_node(consequence, macro_names);
            if let Some(alt) = alternative {
                lower_node(alt, macro_names);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            lower_node(condition, macro_names);
            lower_node(body, macro_names);
        }
        Node::ForInStatement { iterable, body, .. } => {
            lower_node(iterable, macro_names);
            lower_node(body, macro_names);
        }
        Node::InfixExpression { left, right, .. } => {
            lower_node(left, macro_names);
            lower_node(right, macro_names);
        }
        Node::PrefixExpression { right, .. } => lower_node(right, macro_names),
        Node::FieldAccess { target, .. } => lower_node(target, macro_names),
        Node::FieldAssignment { target, value, .. } => {
            lower_node(target, macro_names);
            lower_node(value, macro_names);
        }
        Node::IndexExpression { target, index, .. } => {
            lower_node(target, macro_names);
            lower_node(index, macro_names);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items.iter_mut() {
                lower_node(i, macro_names);
            }
        }
        _ => {}
    }
}

/// Serialize an expression node back to a minimal source string so it
/// can be substituted into a macro expansion template.
///
/// Handles the common cases used in macro arguments. Complex sub-
/// expressions fall back to a placeholder that will produce a parse
/// error in the expansion, making the failure explicit.
///
/// RES-2332: routes recursion through `write_node_source(node, &mut
/// String)` so the entire serialization is built into a single shared
/// buffer. The previous shape allocated one intermediate `String` per
/// interior node (InfixExpression / PrefixExpression / FieldAccess
/// via `format!`) plus a `Vec<String>` + `.join(", ")` for every
/// `CallExpression`. For a deeply-nested macro argument, that's
/// O(depth) wasted heap allocations per `node_to_source` call.
/// Mirrors RES-2268 (recovers_to_bmc::node_to_smtlib2), RES-2270
/// (behavioral_fingerprint::node_text), RES-2272 (lint::clause_text),
/// RES-2276 / RES-2278 (verifier render_clause).
fn node_to_source(node: &Node) -> String {
    let mut out = String::new();
    write_node_source(node, &mut out);
    out
}

fn write_node_source(node: &Node, out: &mut String) {
    use std::fmt::Write as _;
    match node {
        Node::IntegerLiteral { value, .. } => {
            let _ = write!(out, "{}", value);
        }
        Node::FloatLiteral { value, .. } => {
            let _ = write!(out, "{}", value);
        }
        Node::BooleanLiteral { value, .. } => {
            let _ = write!(out, "{}", value);
        }
        Node::StringLiteral { value, .. } => {
            out.push('"');
            for c in value.chars() {
                if c == '"' {
                    out.push_str("\\\"");
                } else {
                    out.push(c);
                }
            }
            out.push('"');
        }
        Node::Identifier { name, .. } => out.push_str(name),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            out.push('(');
            write_node_source(left, out);
            out.push(' ');
            out.push_str(operator);
            out.push(' ');
            write_node_source(right, out);
            out.push(')');
        }
        Node::PrefixExpression {
            operator, right, ..
        } => {
            out.push_str(operator);
            write_node_source(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            write_node_source(function, out);
            out.push('(');
            for (i, a) in arguments.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_node_source(a, out);
            }
            out.push(')');
        }
        Node::FieldAccess { target, field, .. } => {
            write_node_source(target, out);
            out.push('.');
            out.push_str(field);
        }
        _ => out.push_str("__macro_arg__"),
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case — see RES-1302
    // for the wipe-on-empty race rationale; same pattern saves a
    // wasted RwLock write per compile in the common case.
    let macros = collect();
    if macros.is_empty() {
        return Ok(());
    }
    install(macros);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_assert_eq() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "assert_eq",
            crate::feature_attrs::AttrRecord {
                name: "macro".into(),
                args: r#"pattern = "$1, $2", expansion = "if $1 != $2 { panic(\"not equal\") }""#
                    .into(),
                line: 0,
            },
        );
        install(collect());
        let exp = expand("assert_eq", &["x".into(), "5".into()]).unwrap();
        assert!(exp.contains("if x != 5"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn node_to_source_handles_integer() {
        let n = Node::IntegerLiteral {
            value: 42,
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "42");
    }

    #[test]
    fn node_to_source_handles_bool() {
        let n = Node::BooleanLiteral {
            value: true,
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "true");
    }

    #[test]
    fn node_to_source_handles_string_escaping() {
        let n = Node::StringLiteral {
            value: r#"say "hi""#.to_string(),
            span: Default::default(),
        };
        let s = node_to_source(&n);
        assert!(s.starts_with('"') && s.ends_with('"'));
        assert!(s.contains("\\\""));
    }

    #[test]
    fn node_to_source_handles_identifier() {
        let n = Node::Identifier {
            name: "foo".into(),
            span: Default::default(),
        };
        assert_eq!(node_to_source(&n), "foo");
    }

    #[test]
    fn lower_program_is_noop_when_no_macros() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Clear MACROS registry.
        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }
        let mut program = Node::Program(vec![]);
        lower_program(&mut program); // must not panic
        crate::feature_attrs::reset();
    }

    #[test]
    fn lower_program_expands_call_to_registered_macro() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();

        // Register a trivial identity macro: `id(x)` → `x`
        install(vec![MacroDef {
            name: "id".into(),
            pattern: "$1".into(),
            expansion: "$1".into(),
        }]);

        // Build a minimal program: `id(99)`
        let call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "id".into(),
                span: Default::default(),
            }),
            arguments: vec![Node::IntegerLiteral {
                value: 99,
                span: Default::default(),
            }],
            span: Default::default(),
        };
        let mut program = Node::Program(vec![crate::Spanned {
            node: Node::ExpressionStatement {
                expr: Box::new(call),
                span: Default::default(),
            },
            span: Default::default(),
        }]);

        lower_program(&mut program);

        // After lowering, the ExpressionStatement's expr should be
        // IntegerLiteral(99), not CallExpression.
        if let Node::Program(stmts) = &program {
            if let Node::ExpressionStatement { expr, .. } = &stmts[0].node {
                assert!(
                    matches!(expr.as_ref(), Node::IntegerLiteral { value: 99, .. }),
                    "expected IntegerLiteral(99), got: {:?}",
                    expr
                );
            } else {
                panic!("expected ExpressionStatement");
            }
        } else {
            panic!("expected Program");
        }

        // Cleanup.
        if let Ok(mut g) = MACROS.write() {
            g.clear();
        }
        crate::feature_attrs::reset();
    }
}
