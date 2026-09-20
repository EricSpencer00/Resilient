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
    // RES-1306: gate `install` on the non-empty case. The previous
    // wiring did the RwLock write before the `is_empty` early-out,
    // burning a write-lock acquisition + replace on every program
    // that declares no `#[async_fn]` attribute (the overwhelming
    // majority). It also created the same wipe-on-empty test race
    // documented in RES-1302 against any test that installs into
    // `ASYNC_FNS` directly. Bail when the collected set is empty.
    let async_fns = collect();
    if async_fns.is_empty() {
        return Ok(());
    }
    // RES-1487: validate before `install` so `async_fns` moves into
    // install instead of cloning. The previous shape did
    // `install(async_fns.clone())` up front; the validation loop
    // borrowed `&async_fns` and ran after. Reorder so install takes
    // ownership at the end of the success path. Same shape as
    // RES-1481 (derives) / RES-1485 (recursive_types).
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if !async_fns.contains(name) {
                // RES-1445: collect leaks as `&str` borrows from the
                // AST instead of cloning callee names into owned
                // Strings. Only the first leak (`leaks[0]`) makes it
                // into the error message. Same shape as RES-1439 /
                // RES-1441.
                let mut leaks: Vec<&str> = Vec::new();
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
    install(async_fns);
    Ok(())
}

fn walk_async_calls<'a>(node: &'a Node, async_fns: &HashSet<String>, out: &mut Vec<&'a str>) {
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
                    out.push(name.as_str());
                }
            }
            walk_async_calls(function, async_fns, out);
            for a in arguments {
                walk_async_calls(a, async_fns, out);
            }
        }
        Node::Program(stmts) => {
            for stmt in stmts {
                walk_async_calls(&stmt.node, async_fns, out);
            }
        }
        Node::Function {
            defaults,
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            for default in defaults.iter().flatten() {
                walk_async_calls(default, async_fns, out);
            }
            for clause in requires {
                walk_async_calls(clause, async_fns, out);
            }
            walk_async_calls(body, async_fns, out);
            for clause in ensures {
                walk_async_calls(clause, async_fns, out);
            }
            if let Some(clause) = recovers_to {
                walk_async_calls(clause, async_fns, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_async_calls(s, async_fns, out);
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            walk_async_calls(body, async_fns, out);
            for invariant in invariants {
                walk_async_calls(invariant, async_fns, out);
            }
            if let Some(timeout) = timeout {
                walk_async_calls(timeout, async_fns, out);
            }
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            walk_async_calls(condition, async_fns, out);
            if let Some(message) = message {
                walk_async_calls(message, async_fns, out);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::NewtypeConstruct { value, .. }
        | Node::NamedArg { value, .. }
        | Node::DeferStatement { expr: value, .. }
        | Node::InvariantStatement { expr: value, .. }
        | Node::BreakWith { value, .. } => walk_async_calls(value, async_fns, out),
        Node::LetDestructureStruct { value, .. } | Node::LetTupleDestructure { value, .. } => {
            walk_async_calls(value, async_fns, out)
        }
        Node::ReturnStatement {
            value: Some(value), ..
        } => walk_async_calls(value, async_fns, out),
        Node::ReturnStatement { value: None, .. } => {}
        Node::ExpressionStatement { expr, .. } => walk_async_calls(expr, async_fns, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_async_calls(condition, async_fns, out);
            walk_async_calls(consequence, async_fns, out);
            if let Some(alternative) = alternative {
                walk_async_calls(alternative, async_fns, out);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            walk_async_calls(condition, async_fns, out);
            for invariant in invariants {
                walk_async_calls(invariant, async_fns, out);
            }
            walk_async_calls(body, async_fns, out);
        }
        Node::ForInStatement {
            iterable,
            body,
            invariants,
            ..
        } => {
            walk_async_calls(iterable, async_fns, out);
            for invariant in invariants {
                walk_async_calls(invariant, async_fns, out);
            }
            walk_async_calls(body, async_fns, out);
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                walk_async_calls(stmt, async_fns, out);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    walk_async_calls(stmt, async_fns, out);
                }
            }
        }
        Node::TryExpression { expr, .. } => walk_async_calls(expr, async_fns, out),
        Node::OptionalChain { object, access, .. } => {
            walk_async_calls(object, async_fns, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for arg in args {
                    walk_async_calls(arg, async_fns, out);
                }
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            for clause in requires {
                walk_async_calls(clause, async_fns, out);
            }
            walk_async_calls(body, async_fns, out);
            for clause in ensures {
                walk_async_calls(clause, async_fns, out);
            }
            if let Some(clause) = recovers_to {
                walk_async_calls(clause, async_fns, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_async_calls(scrutinee, async_fns, out);
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    walk_async_calls(guard, async_fns, out);
                }
                walk_async_calls(body, async_fns, out);
            }
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                walk_async_calls(base, async_fns, out);
            }
            for (_, value) in fields {
                walk_async_calls(value, async_fns, out);
            }
        }
        Node::FieldAccess { target, .. } => walk_async_calls(target, async_fns, out),
        Node::FieldAssignment { target, value, .. } => {
            walk_async_calls(target, async_fns, out);
            walk_async_calls(value, async_fns, out);
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                walk_async_calls(item, async_fns, out);
            }
        }
        Node::IndexExpression { target, index, .. } => {
            walk_async_calls(target, async_fns, out);
            walk_async_calls(index, async_fns, out);
        }
        Node::Slice { target, lo, hi, .. } => {
            walk_async_calls(target, async_fns, out);
            if let Some(lo) = lo {
                walk_async_calls(lo, async_fns, out);
            }
            if let Some(hi) = hi {
                walk_async_calls(hi, async_fns, out);
            }
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            walk_async_calls(target, async_fns, out);
            walk_async_calls(index, async_fns, out);
            walk_async_calls(value, async_fns, out);
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                walk_async_calls(key, async_fns, out);
                walk_async_calls(value, async_fns, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            walk_async_calls(left, async_fns, out);
            walk_async_calls(right, async_fns, out);
        }
        Node::PrefixExpression { right, .. } => walk_async_calls(right, async_fns, out),
        Node::TupleIndex { tuple, .. } => walk_async_calls(tuple, async_fns, out),
        Node::Range { lo, hi, .. } => {
            walk_async_calls(lo, async_fns, out);
            walk_async_calls(hi, async_fns, out);
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    walk_async_calls(expr, async_fns, out);
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    walk_async_calls(lo, async_fns, out);
                    walk_async_calls(hi, async_fns, out);
                }
                crate::quantifiers::QuantRange::Iterable(iterable) => {
                    walk_async_calls(iterable, async_fns, out)
                }
            }
            walk_async_calls(body, async_fns, out);
        }
        Node::UnsafeBlock { body, .. } | Node::BenchBlock { body, .. } => {
            walk_async_calls(body, async_fns, out)
        }
        Node::ModuleDecl { body, .. } => {
            for item in body {
                walk_async_calls(item, async_fns, out);
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

    #[test]
    fn async_to_async_call_allowed() {
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
        crate::feature_attrs::record(
            "fetch_all",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn fetch(int x) -> int { return x; }
            fn fetch_all(int y) -> int { return fetch(y); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_async_calls_from_sync_fails() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "a",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "b",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn a(int x) -> int { return x; }
            fn b(int x) -> int { return x + 1; }
            fn sync_caller(int x) -> int { a(x); b(x); return x; }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn nested_block_on_allowed() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "async_fn",
            crate::feature_attrs::AttrRecord {
                name: "async_fn".into(),
                args: String::new(),
                line: 0,
            },
        );
        let src = r#"
            fn async_fn(int x) -> int { return x; }
            fn sync_caller(int x) -> int { return block_on(block_on(async_fn(x))); }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn sync_calls_hidden_in_control_flow_are_rejected() {
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
            fn caller(int x) -> int {
                if x > 0 { fetch(x); }
                while x > 0 { fetch(x); break; }
                for item in [x] { fetch(item); }
                return match x { _ => fetch(x) };
            }
        "#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn sync_calls_hidden_in_try_catch_and_defer_are_rejected() {
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
            fn caller(int x) -> int {
                try { fetch(x); } catch Timeout { return x; }
                defer fetch(x);
                return x;
            }
        "#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn sync_calls_inside_nested_expressions_are_rejected() {
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
            fn caller(int x) -> int {
                return 1 + fetch(x);
            }
        "#;
        let (prog, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(check(&prog, "test").is_err());
        crate::feature_attrs::reset();
    }
}
