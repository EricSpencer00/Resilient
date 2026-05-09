//! Feature 24/50 — Ghost Types / Specification-Only Code.
//!
//! `#[ghost]` on a function marks it as specification-only: the body
//! exists for verification but is fully erased at runtime. Calls to
//! ghost fns from non-ghost code are rejected — they would force the
//! compiler to retain the ghost body, defeating the purpose.
//!
//! Ghost fns are typically used to express invariants in a richer
//! language than `requires`/`ensures` allows: e.g.,
//! `ghost fn sorted(Array<int> a) -> bool { ... }` and then
//! `ensures sorted(result)`.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;
use std::sync::RwLock;

static GHOST_FNS: RwLock<Option<HashSet<String>>> = RwLock::new(None);

pub fn collect() -> HashSet<String> {
    crate::feature_attrs::find_kind("ghost")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

pub fn install(set: HashSet<String>) {
    if let Ok(mut g) = GHOST_FNS.write() {
        *g = Some(set);
    }
}

pub fn is_ghost(name: &str) -> bool {
    GHOST_FNS
        .read()
        .ok()
        .and_then(|g| g.clone())
        .map(|s| s.contains(name))
        .unwrap_or(false)
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let ghosts = collect();
    install(ghosts.clone());
    if ghosts.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if !ghosts.contains(name) {
                let mut leaks = Vec::new();
                walk_calls(body, &ghosts, &mut leaks);
                if !leaks.is_empty() {
                    return Err(format!(
                        "{}:0:0: error: non-ghost fn `{}` calls ghost fn `{}` — ghost code cannot be reached at runtime",
                        source_path, name, leaks[0]
                    ));
                }
            }
        }
    }
    Ok(())
}

fn walk_calls(node: &Node, ghosts: &HashSet<String>, out: &mut Vec<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if ghosts.contains(name) {
                    out.push(name.clone());
                }
            }
            for a in arguments {
                walk_calls(a, ghosts, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_calls(s, ghosts, out);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => walk_calls(e, ghosts, out),
        Node::LetStatement { value, .. } => walk_calls(value, ghosts, out),
        Node::ExpressionStatement { expr, .. } => walk_calls(expr, ghosts, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_calls(condition, ghosts, out);
            walk_calls(consequence, ghosts, out);
            if let Some(e) = alternative {
                walk_calls(e, ghosts, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn ghost_call_from_runtime_is_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "spec_sorted",
            crate::feature_attrs::AttrRecord {
                name: "ghost".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn spec_sorted(int x) -> bool { return true; }
            fn runtime(int x) -> bool { return spec_sorted(x); }
        "#;
        let (prog, _) = parse(src);
        let r = check(&prog, "test");
        assert!(r.is_err(), "expected runtime call to ghost fn to error");
        crate::feature_attrs::reset();
    }
}
