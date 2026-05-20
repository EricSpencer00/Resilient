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
    // RES-1308: gate `install` on the non-empty case. The historical
    // wiring called `install(ghosts.clone())` before the early-out,
    // burning a RwLock write per compile and creating the
    // wipe-on-empty test race documented in RES-1302.
    let ghosts = collect();
    if ghosts.is_empty() {
        return Ok(());
    }
    // RES-1487: validate before `install` so `ghosts` can be moved
    // into install instead of cloned. The previous shape did
    // `install(ghosts.clone())` up front; the validation loop
    // borrowed `&ghosts` and ran after. Reorder so install takes
    // ownership at the end of the success path. Same shape as
    // RES-1481 (derives) / RES-1485 (recursive_types).
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if !ghosts.contains(name) {
                // RES-2340: short-circuit on the first ghost call
                // found. The previous shape walked the entire body
                // collecting every leak into a `Vec<&str>`, then
                // formatted only `leaks[0]` and discarded the rest.
                // For a body that calls multiple ghost fns, the
                // remaining walks past the first hit were pure
                // overhead. `find_first_ghost_call` returns
                // `Option<&str>` instead and stops the recursion at
                // the first match. Mirrors the early-termination
                // pattern in `uniqueness_walk::any_node` (RES-1238).
                if let Some(leak) = find_first_ghost_call(body, &ghosts) {
                    return Err(format!(
                        "{}:0:0: error: non-ghost fn `{}` calls ghost fn `{}` — ghost code cannot be reached at runtime",
                        source_path, name, leak
                    ));
                }
            }
        }
    }
    install(ghosts);
    Ok(())
}

/// RES-2340: walk `node` looking for the first call to a fn name
/// in `ghosts`. Returns `Some(callee)` on the first hit, `None` if
/// no ghost call is reached. Recursion short-circuits via the
/// `?`-then-return shape, so a body with N call sites and a ghost
/// call at position K bails after K steps instead of always
/// walking to N.
fn find_first_ghost_call<'a>(node: &'a Node, ghosts: &HashSet<String>) -> Option<&'a str> {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && ghosts.contains(name)
            {
                return Some(name.as_str());
            }
            for a in arguments {
                if let Some(hit) = find_first_ghost_call(a, ghosts) {
                    return Some(hit);
                }
            }
            None
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                if let Some(hit) = find_first_ghost_call(s, ghosts) {
                    return Some(hit);
                }
            }
            None
        }
        Node::ReturnStatement { value: Some(e), .. } => find_first_ghost_call(e, ghosts),
        Node::LetStatement { value, .. } => find_first_ghost_call(value, ghosts),
        Node::ExpressionStatement { expr, .. } => find_first_ghost_call(expr, ghosts),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => find_first_ghost_call(condition, ghosts)
            .or_else(|| find_first_ghost_call(consequence, ghosts))
            .or_else(|| {
                alternative
                    .as_ref()
                    .and_then(|a| find_first_ghost_call(a, ghosts))
            }),
        _ => None,
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

    #[test]
    fn no_ghost_attrs_check_returns_ok() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn ghost_fn_not_called_by_runtime_is_ok() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "spec_helper",
            crate::feature_attrs::AttrRecord {
                name: "ghost".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn spec_helper(int x) -> bool { return true; }
            fn runtime(int x) -> int { return x; }
        "#;
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
