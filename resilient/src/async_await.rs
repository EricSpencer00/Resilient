//! Feature 32/50 — Async/Await.
//!
//! `#[async_fn]` marks a function as suspendable. The first slice
//! ships:
//!
//! 1. Recognition: the attribute parser registers async fns.
//! 2. Effect: async fns are required to be called only from other
//!    async fns OR from a `runtime::block_on` builtin.
//! 3. Cooperative scheduler: a tiny round-robin executor in the
//!    runtime that polls registered futures.
//!
//! This is intentionally a first slice. The full continuation
//! transformation (CPS lowering or coroutine-style) is a downstream
//! PR; today, async fns run synchronously but their contract surface
//! exists.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;
use std::sync::RwLock;

static ASYNC_FNS: RwLock<Option<HashSet<String>>> = RwLock::new(None);

pub fn collect() -> HashSet<String> {
    crate::feature_attrs::find_kind("async_fn")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

pub fn install(set: HashSet<String>) {
    if let Ok(mut g) = ASYNC_FNS.write() {
        *g = Some(set);
    }
}

pub fn is_async(name: &str) -> bool {
    ASYNC_FNS
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|s| s.contains(name))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let async_fns = collect();
    install(async_fns.clone());
    if async_fns.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if !async_fns.contains(name) {
                let mut leaks = Vec::new();
                walk_async_calls(body, &async_fns, &mut leaks);
                if !leaks.is_empty() {
                    return Err(format!(
                        "{}:0:0: error: non-async fn `{}` calls async fn `{}` without `block_on`",
                        source_path, name, leaks[0]
                    ));
                }
            }
        }
    }
    Ok(())
}

fn walk_async_calls(node: &Node, async_fns: &HashSet<String>, out: &mut Vec<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if name == "block_on" {
                    // explicit bridge — skip recursion into args here since they are awaited
                    return;
                }
                if async_fns.contains(name) {
                    out.push(name.clone());
                }
            }
            for a in arguments {
                walk_async_calls(a, async_fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_async_calls(s, async_fns, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_async_calls(e, async_fns, out),
        Node::LetStatement { value, .. } => walk_async_calls(value, async_fns, out),
        Node::ExpressionStatement { expr, .. } => walk_async_calls(expr, async_fns, out),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn async_call_from_sync_is_blocked() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fetch",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn fetch(int x) -> int { return x; }
            fn caller(int x) -> int { return fetch(x); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn block_on_bridges_to_sync() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fetch",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn fetch(int x) -> int { return x; }
            fn caller(int x) -> int { return block_on(fetch(x)); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
