//! Feature 16/50 — Information Flow / Non-Interference Types.
//!
//! `#[secret]` marks a function as a *source* of classified data: any
//! value it returns is secret. `#[public]` marks a function as an
//! *observable sink*: the value it returns is visible to an attacker.
//! `#[declassify]` marks a *laundering boundary*: the value a
//! declassify-tagged function returns is treated as public regardless of
//! its arguments — the deliberate, audited downgrade point.
//!
//! The non-interference property this pass enforces is the explicit-flow
//! fragment of *"a `#[public]` function's output must not depend on
//! `#[secret]` data"*: it propagates a **taint set** — the secret
//! sources a value is derived from — through the body of every public
//! function and rejects the function if any value it *returns* is still
//! tainted.
//!
//! ### What counts as a flow
//! - calling a `#[secret]` fn taints the result;
//! - calling a `#[declassify]` fn launders it (the result is public);
//! - calling any other fn passes taint through from its arguments;
//! - `let` / assignment bind a variable to its initializer's taint;
//! - infix / prefix operators, field / index access, and array / tuple
//!   literals propagate taint from their operands;
//! - `if` / `while` / `for` union-merge the taint a branch may assign.
//!
//! A secret value that is *computed and discarded* (never returned) is
//! **not** a leak — that is the precision win over the original
//! call-graph approximation (RES-2824), which flagged any transitive
//! secret call even when the result never reached the public output.
//!
//! ### Out of scope (tracked follow-ups)
//! - **Implicit flows**: a secret-derived branch condition that taints
//!   what the branch assigns or returns. Sound handling needs
//!   program-counter labels.
//! - **Semantic non-interference** via self-composition + Z3
//!   (`#[noninterference(low = …, high = …)]`), the rigorous companion
//!   that *proves* output-independence rather than approximating
//!   explicit flow.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Label {
    Secret,
    Public,
    Unknown,
}

/// Legacy per-item label view, retained for external callers. The taint
/// analysis below reads the function sets directly via [`collect_fns`].
pub fn collect_param_labels() -> HashMap<String, Label> {
    let mut out = HashMap::new();
    for (item, _rec) in crate::feature_attrs::find_kind("secret") {
        out.insert(item, Label::Secret);
    }
    for (item, _rec) in crate::feature_attrs::find_kind("public") {
        out.insert(item, Label::Public);
    }
    out
}

/// All function names carrying the attribute `kind` (`secret`, `public`,
/// or `declassify`).
fn collect_fns(kind: &str) -> HashSet<String> {
    crate::feature_attrs::find_kind(kind)
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

/// The set of `#[secret]` sources a value is derived from. Empty == the
/// value is public. A `BTreeSet` keeps the diagnostic's source list
/// deterministic.
type Taint = BTreeSet<String>;

/// The information-flow classification of every tagged function.
struct Classes {
    secret: HashSet<String>,
    declassify: HashSet<String>,
}

/// Taint of an expression's *value*, given the current variable env.
fn expr_taint(node: &Node, env: &HashMap<String, Taint>, cls: &Classes) -> Taint {
    match node {
        Node::Identifier { name, .. } => env.get(name).cloned().unwrap_or_default(),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if cls.declassify.contains(name) {
                    // Laundered at the audited downgrade point: the result
                    // is public no matter how secret the arguments were.
                    return Taint::new();
                }
                if cls.secret.contains(name) {
                    // Secret source: its result is classified regardless
                    // of the arguments.
                    let mut t = Taint::new();
                    t.insert(name.clone());
                    return t;
                }
            }
            // Ordinary callee (user fn, builtin, or a closure held in a
            // variable): conservatively pass taint through from the
            // arguments and the callee expression itself.
            let mut t = expr_taint(function, env, cls);
            for a in arguments {
                t.extend(expr_taint(a, env, cls));
            }
            t
        }
        Node::InfixExpression { left, right, .. } => {
            let mut t = expr_taint(left, env, cls);
            t.extend(expr_taint(right, env, cls));
            t
        }
        Node::PrefixExpression { right, .. } => expr_taint(right, env, cls),
        Node::FieldAccess { target, .. } => expr_taint(target, env, cls),
        Node::IndexExpression { target, index, .. } => {
            let mut t = expr_taint(target, env, cls);
            t.extend(expr_taint(index, env, cls));
            t
        }
        Node::TryExpression { expr, .. } => expr_taint(expr, env, cls),
        Node::ArrayLiteral { items, .. } | Node::TupleLiteral { items, .. } => {
            let mut t = Taint::new();
            for it in items {
                t.extend(expr_taint(it, env, cls));
            }
            t
        }
        // Literals and constructs that do not carry a secret-derived
        // scalar contribute no taint in this explicit-flow slice.
        _ => Taint::new(),
    }
}

/// Union the taint of every binding in `from` into `into`. Used to merge
/// the environments of conditionally-executed branches back into the
/// surrounding scope (sound over-approximation).
fn merge_env(into: &mut HashMap<String, Taint>, from: &HashMap<String, Taint>) {
    for (k, v) in from {
        into.entry(k.clone()).or_default().extend(v.iter().cloned());
    }
}

/// Walk a statement, updating `env` and pushing the taint of every
/// returned value into `returns`.
fn walk_stmt(
    node: &Node,
    env: &mut HashMap<String, Taint>,
    cls: &Classes,
    returns: &mut Vec<Taint>,
) {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_stmt(s, env, cls, returns);
            }
        }
        Node::LetStatement { name, value, .. } | Node::Assignment { name, value, .. } => {
            let t = expr_taint(value, env, cls);
            env.insert(name.clone(), t);
        }
        Node::ReturnStatement { value: Some(e), .. } => {
            returns.push(expr_taint(e, env, cls));
        }
        Node::ExpressionStatement { expr, .. } => {
            // Evaluated for effect; the value is discarded, so it cannot
            // reach the public output. (Implicit / side-channel flows are
            // out of scope for this slice.)
            let _ = expr_taint(expr, env, cls);
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let mut then_env = env.clone();
            walk_stmt(consequence, &mut then_env, cls, returns);
            if let Some(alt) = alternative {
                let mut else_env = env.clone();
                walk_stmt(alt, &mut else_env, cls, returns);
                merge_env(env, &else_env);
            }
            merge_env(env, &then_env);
        }
        Node::WhileStatement { body, .. } => {
            let mut body_env = env.clone();
            walk_stmt(body, &mut body_env, cls, returns);
            merge_env(env, &body_env);
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            ..
        } => {
            let it = expr_taint(iterable, env, cls);
            let mut body_env = env.clone();
            body_env.insert(name.clone(), it);
            walk_stmt(body, &mut body_env, cls, returns);
            merge_env(env, &body_env);
        }
        _ => {}
    }
}

/// Return one diagnostic per `#[public]` function whose returned value is
/// secret-derived (an explicit-flow leak). Empty when there is no leak.
pub fn check_program(program: &Node) -> Vec<String> {
    let secret = collect_fns("secret");
    let public = collect_fns("public");
    // Fast-reject: a leak can only fire inside a public-fn body that can
    // reach a secret source.
    if secret.is_empty() || public.is_empty() {
        return Vec::new();
    }
    let cls = Classes {
        secret,
        declassify: collect_fns("declassify"),
    };
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    let mut errors = Vec::new();
    for s in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            ..
        } = &s.node
        {
            if !public.contains(name) {
                continue;
            }
            // A public function's parameters are public inputs: untainted.
            let mut env: HashMap<String, Taint> = parameters
                .iter()
                .map(|(_ty, pname)| (pname.clone(), Taint::new()))
                .collect();
            let mut returns = Vec::new();
            walk_stmt(body, &mut env, &cls, &mut returns);
            let mut leaked = Taint::new();
            for r in returns {
                leaked.extend(r);
            }
            if !leaked.is_empty() {
                let srcs = leaked.into_iter().collect::<Vec<_>>().join("`, `");
                errors.push(format!(
                    "info-flow: `{name}` is `#[public]` but returns secret data from `#[secret]` fn `{srcs}` — route it through a `#[declassify]` fn to launder it"
                ));
            }
        }
    }
    errors
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let errs = check_program(program);
    if !errs.is_empty() {
        return Err(format!("{}:0:0: error: {}", source_path, errs[0]));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// Tag `item` with attribute `kind` in the shared registry.
    fn tag(item: &str, kind: &str) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: kind.into(),
                args: String::new(),
                line: 0,
            },
        );
    }

    #[test]
    fn secret_to_public_is_blocked() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { return leak(x); }
        "#;
        let (prog, _) = parse(src);
        assert!(!check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn discarded_secret_is_not_a_leak() {
        // RES-2824: the precision win. Calling a secret fn and discarding
        // the result does not leak it to the public output.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { leak(x); return 0; }
        "#;
        let (prog, _) = parse(src);
        assert!(
            check_program(&prog).is_empty(),
            "discarded secret value must not be flagged"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn declassified_secret_is_clean() {
        // RES-2824: routing the secret through a #[declassify] fn launders
        // it, so the public sink may return it.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("redact", "declassify");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn redact(int x) -> int { return 0; }
            fn log(int x) -> int { return redact(leak(x)); }
        "#;
        let (prog, _) = parse(src);
        assert!(
            check_program(&prog).is_empty(),
            "declassified value must be clean"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn taint_through_let_leaks() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { let s = leak(x); return s; }
        "#;
        let (prog, _) = parse(src);
        assert!(!check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn taint_through_operator_leaks() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { return leak(x) + 1; }
        "#;
        let (prog, _) = parse(src);
        assert!(!check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn declassify_through_let_is_clean() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("redact", "declassify");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn redact(int x) -> int { return 0; }
            fn log(int x) -> int { let s = leak(x); let p = redact(s); return p; }
        "#;
        let (prog, _) = parse(src);
        assert!(check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn diagnostic_names_the_secret_source() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("leak", "secret");
        tag("log", "public");
        let src = r#"
            fn leak(int x) -> int { return x; }
            fn log(int x) -> int { return leak(x); }
        "#;
        let (prog, _) = parse(src);
        let errs = check_program(&prog);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("`log`") && errs[0].contains("`leak`"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_program_no_attrs_returns_empty() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check_program(&prog).is_empty());
        crate::feature_attrs::reset();
    }
}
