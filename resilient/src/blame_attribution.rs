//! Feature 7/50 — Blame Attribution.
//!
//! When a `requires` clause is violated at runtime, the standard
//! diagnostic identifies the *callee* whose precondition wasn't
//! met. That's only half the story — the bug is usually at the
//! *caller*, who passed bad arguments.
//!
//! Blame Attribution maintains a small static graph from each
//! `requires` clause to every call site that supplies its arguments.
//! When a precondition fails (runtime path), the diagnostic walks the
//! call graph one level up and names the responsible caller.
//!
//! This module owns the static analysis: it builds the
//! `requires_var → caller_set` map at typecheck time and exposes a
//! `lookup(callee, param_name)` API the runtime error path consults.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct BlameMap {
    /// Key: callee fn name. Value: list of (caller_name, arg_index)
    /// pairs that pass arguments into that fn.
    pub edges: HashMap<String, Vec<(String, usize)>>,
}

static BLAME_MAP: RwLock<Option<BlameMap>> = RwLock::new(None);

pub fn build(program: &Node) -> BlameMap {
    let mut map = BlameMap::default();
    let Node::Program(stmts) = program else {
        return map;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            walk(body, name, &mut map);
        }
    }
    map
}

fn walk(node: &Node, caller: &str, map: &mut BlameMap) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name: callee, .. } = function.as_ref() {
                let entry = map.edges.entry(callee.clone()).or_default();
                for (idx, _) in arguments.iter().enumerate() {
                    entry.push((caller.to_string(), idx));
                }
            }
            for a in arguments {
                walk(a, caller, map);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, caller, map);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, caller, map);
            walk(consequence, caller, map);
            if let Some(e) = alternative {
                walk(e, caller, map);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk(condition, caller, map);
            walk(body, caller, map);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk(iterable, caller, map);
            walk(body, caller, map);
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            walk(value, caller, map)
        }
        Node::ReturnStatement { value: Some(e), .. } => walk(e, caller, map),
        Node::ExpressionStatement { expr, .. } => walk(expr, caller, map),
        Node::InfixExpression { left, right, .. } => {
            walk(left, caller, map);
            walk(right, caller, map);
        }
        Node::PrefixExpression { right, .. } => walk(right, caller, map),
        _ => {}
    }
}

pub fn install(map: BlameMap) {
    if let Ok(mut g) = BLAME_MAP.write() {
        *g = Some(map);
    }
}

pub fn callers_of(callee: &str) -> Vec<(String, usize)> {
    BLAME_MAP
        .read()
        .ok()
        .and_then(|g| g.clone())
        .and_then(|m| m.edges.get(callee).cloned())
        .unwrap_or_default()
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1291: fast-reject. `build` walks every function body
    // recursively, emitting one (caller, arg_index) edge per
    // `CallExpression`. For programs with zero `CallExpression`
    // anywhere, the walk visits every Node but emits nothing. Pre-
    // scan with the early-terminating `any_node` (RES-1238) and skip
    // the walk when no `CallExpression` exists. We still call
    // `install` with an empty `BlameMap` so the process-global
    // `BLAME_MAP` is reset between compilations and `callers_of(...)`
    // doesn't return stale entries from a prior program.
    let has_call =
        crate::uniqueness_walk::any_node(program, |n| matches!(n, Node::CallExpression { .. }));
    if !has_call {
        install(BlameMap::default());
        return Ok(());
    }
    install(build(program));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn caller_is_attributed() {
        let src = r#"
            fn add(int a, int b) -> int requires b != 0 { return a + b; }
            fn main(int dummy) { let x = add(1, 2); return 0; }
        "#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        let edges = map.edges.get("add").expect("add should have a caller");
        assert!(edges.iter().any(|(c, _)| c == "main"));
    }

    #[test]
    fn install_and_lookup_works() {
        let src = r#"
            fn helper(int x) { return x; }
            fn caller(int dummy) { let r = helper(42); return r; }
        "#;
        let (prog, _) = parse(src);
        let _ = check(&prog, "test");
        let callers = callers_of("helper");
        assert!(!callers.is_empty());
    }

    #[test]
    fn no_calls_no_blame() {
        let src = r#"fn solo(int x) { return x; }"#;
        let (prog, _) = parse(src);
        let map = build(&prog);
        assert!(!map.edges.contains_key("solo"));
    }
}
