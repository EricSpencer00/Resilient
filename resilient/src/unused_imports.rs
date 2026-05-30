//! RES-2590: Warn on unused `use` imports.
//!
//! Emits a warning to stderr for any `use "path" as alias;` import
//! where the alias namespace (`alias::`) is never referenced in the
//! program.
//!
//! ## Scope
//!
//! Only **aliased** file imports are checked:
//! ```text
//! use "math.rz" as math;   ← checked — is `math::` used?
//! use "util.rz";           ← skipped (conservative; flat merge)
//! use std::http;           ← skipped (stdlib)
//! use std::http as h;      ← skipped (stdlib)
//! ```
//!
//! For `use "path"` without an alias, the imported declarations are
//! spliced flat into the current scope — their names are
//! indistinguishable from locally-defined names without resolving
//! the import, so the check skips them to avoid false positives.
//!
//! ## When to call
//!
//! This pass must be invoked **before** `imports::expand_uses_with_std`
//! is called, because that function replaces `Node::Use` nodes with the
//! imported content. After expansion, no `Node::Use` nodes remain in
//! the AST and the check has nothing to inspect.
//!
//! See the call site in `lib.rs` near the `has_use` gate.

use crate::string_interp::StringPart;
use crate::{Node, span};
use std::collections::HashSet;

/// Check `program` for unused aliased imports and emit `warning:` lines
/// to stderr for each one.
///
/// `source_path` is used for the diagnostic location string.
///
/// This function never fails — it is a warning-only pass.
pub(crate) fn check(program: &Node, source_path: &str) {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return,
    };

    // Fast-reject: no Use nodes at all.
    if !stmts.iter().any(|s| matches!(&s.node, Node::Use { .. })) {
        return;
    }

    struct ImportInfo<'a> {
        alias: &'a str,
        path: &'a str,
        span: span::Span,
    }

    // Collect aliased imports that are candidates for the warning.
    // Only file-path imports with an explicit alias are checked;
    // stdlib (`std::`) imports are always skipped.
    let mut candidates: Vec<ImportInfo<'_>> = Vec::new();
    for stmt in stmts {
        if let Node::Use { path, alias, span } = &stmt.node {
            if path.starts_with("std::") {
                continue;
            }
            if let Some(alias_str) = alias {
                candidates.push(ImportInfo {
                    alias: alias_str.as_str(),
                    path: path.as_str(),
                    span: *span,
                });
            }
            // Plain `use "file.rz"` without alias: conservative skip.
        }
    }

    if candidates.is_empty() {
        return;
    }

    // Collect all namespace prefixes that actually appear in the code.
    // An identifier like `math::sqrt` contributes prefix `math`.
    let mut used_namespaces: HashSet<&str> = HashSet::new();
    for stmt in stmts {
        collect_namespaces(&stmt.node, &mut used_namespaces);
    }

    // Warn for every candidate whose alias isn't referenced.
    for info in &candidates {
        if !used_namespaces.contains(info.alias) {
            eprintln!(
                "warning: {}:{}:{}: unused import: \"{}\" (alias `{}`)",
                source_path, info.span.start.line, info.span.start.column, info.path, info.alias,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Internal AST walker
// ---------------------------------------------------------------------------

/// Recursively walk `node` and insert any namespace prefix (the part
/// before `::`) from qualified `Node::Identifier` nodes into `out`.
fn collect_namespaces<'a>(node: &'a Node, out: &mut HashSet<&'a str>) {
    match node {
        Node::Identifier { name, .. } => {
            if let Some(idx) = name.find("::") {
                out.insert(&name[..idx]);
            }
        }

        Node::Program(stmts) => {
            for s in stmts {
                collect_namespaces(&s.node, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_namespaces(s, out);
            }
        }
        Node::Function {
            body,
            defaults,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            collect_namespaces(body, out);
            for d in defaults.iter().flatten() {
                collect_namespaces(d, out);
            }
            for r in requires {
                collect_namespaces(r, out);
            }
            for e in ensures {
                collect_namespaces(e, out);
            }
            if let Some(rt) = recovers_to {
                collect_namespaces(rt, out);
            }
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            collect_namespaces(body, out);
            for r in requires {
                collect_namespaces(r, out);
            }
            for e in ensures {
                collect_namespaces(e, out);
            }
            if let Some(rt) = recovers_to {
                collect_namespaces(rt, out);
            }
        }
        Node::LetStatement { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::StaticLet { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::Const { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::Assignment { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(v) = value {
                collect_namespaces(v, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            collect_namespaces(expr, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_namespaces(function, out);
            for a in arguments {
                collect_namespaces(a, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_namespaces(condition, out);
            collect_namespaces(consequence, out);
            if let Some(e) = alternative {
                collect_namespaces(e, out);
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            collect_namespaces(condition, out);
            collect_namespaces(body, out);
            for inv in invariants {
                collect_namespaces(inv, out);
            }
        }
        Node::ForInStatement {
            iterable,
            body,
            invariants,
            ..
        } => {
            collect_namespaces(iterable, out);
            collect_namespaces(body, out);
            for inv in invariants {
                collect_namespaces(inv, out);
            }
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            collect_namespaces(body, out);
            for inv in invariants {
                collect_namespaces(inv, out);
            }
            if let Some(t) = timeout {
                collect_namespaces(t, out);
            }
        }
        Node::PrefixExpression { right, .. } => {
            collect_namespaces(right, out);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_namespaces(left, out);
            collect_namespaces(right, out);
        }
        Node::TryExpression { expr, .. } => {
            collect_namespaces(expr, out);
        }
        Node::OptionalChain { object, .. } => {
            collect_namespaces(object, out);
        }
        Node::IndexExpression { target, index, .. } => {
            collect_namespaces(target, out);
            collect_namespaces(index, out);
        }
        Node::Slice { target, lo, hi, .. } => {
            collect_namespaces(target, out);
            if let Some(l) = lo {
                collect_namespaces(l, out);
            }
            if let Some(h) = hi {
                collect_namespaces(h, out);
            }
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_namespaces(target, out);
            collect_namespaces(index, out);
            collect_namespaces(value, out);
        }
        Node::FieldAccess { target, .. } => {
            collect_namespaces(target, out);
        }
        Node::FieldAssignment { target, value, .. } => {
            collect_namespaces(target, out);
            collect_namespaces(value, out);
        }
        Node::StructLiteral { fields, base, .. } => {
            for (_, v) in fields {
                collect_namespaces(v, out);
            }
            if let Some(b) = base {
                collect_namespaces(b, out);
            }
        }
        Node::LetDestructureStruct { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for e in items {
                collect_namespaces(e, out);
            }
        }
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_namespaces(k, out);
                collect_namespaces(v, out);
            }
        }
        Node::SetLiteral { items, .. } => {
            for e in items {
                collect_namespaces(e, out);
            }
        }
        Node::TupleLiteral { items, .. } => {
            for e in items {
                collect_namespaces(e, out);
            }
        }
        Node::TupleIndex { tuple, .. } => {
            collect_namespaces(tuple, out);
        }
        Node::LetTupleDestructure { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_namespaces(scrutinee, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    collect_namespaces(g, out);
                }
                collect_namespaces(body, out);
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for s in body {
                collect_namespaces(s, out);
            }
            for (_, stmts) in handlers {
                for s in stmts {
                    collect_namespaces(s, out);
                }
            }
        }
        Node::Assert {
            condition, message, ..
        } => {
            collect_namespaces(condition, out);
            if let Some(m) = message {
                collect_namespaces(m, out);
            }
        }
        Node::Assume {
            condition, message, ..
        } => {
            collect_namespaces(condition, out);
            if let Some(m) = message {
                collect_namespaces(m, out);
            }
        }
        Node::InvariantStatement { expr, .. } => {
            collect_namespaces(expr, out);
        }
        Node::Range { lo, hi, .. } => {
            collect_namespaces(lo, out);
            collect_namespaces(hi, out);
        }
        Node::NamedArg { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::InterpolatedString { parts, .. } => {
            for p in parts {
                if let StringPart::Expr(expr) = p {
                    collect_namespaces(expr, out);
                }
            }
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                collect_namespaces(m, out);
            }
        }
        Node::BlanketImpl { methods, .. } => {
            for m in methods {
                collect_namespaces(m, out);
            }
        }
        Node::ModuleDecl { body, .. } => {
            for s in body {
                collect_namespaces(s, out);
            }
        }
        Node::UnsafeBlock { body, .. } => {
            collect_namespaces(body, out);
        }
        Node::Quantifier { body, .. } => {
            collect_namespaces(body, out);
        }
        Node::NewtypeConstruct { value, .. } => {
            collect_namespaces(value, out);
        }
        Node::Actor {
            state_init,
            handlers,
            concurrent_ensures,
            ..
        } => {
            collect_namespaces(state_init, out);
            for h in handlers {
                collect_namespaces(&h.body, out);
            }
            for e in concurrent_ensures {
                collect_namespaces(e, out);
            }
        }
        Node::ActorDecl {
            state_fields,
            always_clauses,
            receive_handlers,
            ..
        } => {
            for (_, _, init) in state_fields {
                collect_namespaces(init, out);
            }
            for c in always_clauses {
                collect_namespaces(c, out);
            }
            for h in receive_handlers {
                collect_namespaces(&h.body, out);
            }
        }
        Node::ClusterDecl { invariants, .. } => {
            for inv in invariants {
                collect_namespaces(inv, out);
            }
        }
        Node::StaticAssert { condition, .. } => {
            collect_namespaces(condition, out);
        }
        // Leaf nodes: literals, declarations without expressions, spans, etc.
        Node::Use { .. }
        | Node::Extern { .. }
        | Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::TypeAlias { .. }
        | Node::RegionDecl { .. }
        | Node::NewtypeDecl { .. }
        | Node::EnumDecl { .. }
        | Node::RegionParam { .. }
        | Node::SupervisorDecl { .. }
        | Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::DurationLiteral { .. }
        | Node::Break { .. }
        | Node::Continue { .. }
        | Node::BreakLabel { .. }
        | Node::ContinueLabel { .. } => {}
        Node::BreakWith { value, .. } => collect_namespaces(value, out),
        Node::DeferStatement { expr, .. } => collect_namespaces(expr, out),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::parse;

    /// Returns the list of warning messages that `check` would emit for
    /// the given source by directly re-implementing the detection logic
    /// (since `check()` prints to stderr, we test through a mirror helper).
    fn warnings_for(src: &str) -> Vec<String> {
        let (program, _errs) = parse(src);
        let stmts = match &program {
            crate::Node::Program(stmts) => stmts,
            _ => return vec![],
        };

        // Collect candidates.
        let mut candidates: Vec<(String, String, crate::span::Span)> = Vec::new();
        for stmt in stmts {
            if let crate::Node::Use { path, alias, span } = &stmt.node {
                if path.starts_with("std::") {
                    continue;
                }
                if let Some(alias_str) = alias {
                    candidates.push((alias_str.clone(), path.clone(), *span));
                }
            }
        }

        if candidates.is_empty() {
            return vec![];
        }

        // Collect used namespaces.
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        for stmt in stmts {
            collect_used(&stmt.node, &mut used);
        }

        candidates
            .iter()
            .filter(|(alias, _, _)| !used.contains(alias.as_str()))
            .map(|(alias, path, span)| {
                format!(
                    "warning: <test>:{}:{}: unused import: \"{}\" (alias `{}`)",
                    span.start.line, span.start.column, path, alias
                )
            })
            .collect()
    }

    fn collect_used(node: &crate::Node, out: &mut std::collections::HashSet<String>) {
        // Use the real walker from the parent module.
        let mut refs: std::collections::HashSet<&str> = std::collections::HashSet::new();
        super::collect_namespaces(node, &mut refs);
        for r in refs {
            out.insert(r.to_string());
        }
    }

    #[test]
    fn no_use_stmts_no_warnings() {
        let warnings = warnings_for("fn main(int _d) {} main();");
        assert!(
            warnings.is_empty(),
            "expected no warnings, got {:?}",
            warnings
        );
    }

    #[test]
    fn aliased_import_used_no_warning() {
        // `math::sqrt` references the `math` namespace — no warning expected.
        let src = "use \"math.rz\" as math; math::sqrt;";
        let warnings = warnings_for(src);
        assert!(
            warnings.is_empty(),
            "used import should not warn, got {:?}",
            warnings
        );
    }

    #[test]
    fn aliased_import_unused_warns() {
        // `use "util.rz" as util;` with no `util::` reference → should warn.
        let src = "use \"util.rz\" as util; let x = 1; x;";
        let warnings = warnings_for(src);
        assert!(!warnings.is_empty(), "unused import should warn");
        assert!(warnings[0].contains("util"), "warning should mention alias");
        assert!(
            warnings[0].contains("util.rz"),
            "warning should mention path"
        );
    }

    #[test]
    fn plain_import_no_alias_skipped() {
        // `use "util.rz"` without an alias: conservative skip.
        let src = "use \"util.rz\"; let x = 1; x;";
        let warnings = warnings_for(src);
        assert!(
            warnings.is_empty(),
            "plain import without alias should not warn"
        );
    }

    #[test]
    fn stdlib_import_skipped() {
        // `use std::http as h;` — stdlib, should never warn.
        let src = "use std::http as h; let x = 1; x;";
        let warnings = warnings_for(src);
        assert!(warnings.is_empty(), "stdlib import should not warn");
    }

    #[test]
    fn multiple_aliases_partial_usage() {
        // `a` is used, `b` is not.
        let src = "use \"a.rz\" as a; use \"b.rz\" as b; a::foo; let x = 1; x;";
        let warnings = warnings_for(src);
        assert_eq!(warnings.len(), 1, "only b should warn, got {:?}", warnings);
        assert!(
            warnings[0].contains("\"b.rz\""),
            "warning should mention b.rz"
        );
    }

    #[test]
    fn stdlib_plain_import_skipped() {
        // `use std::json;` — stdlib plain, should never warn.
        let src = "use std::json; let x = 1; x;";
        let warnings = warnings_for(src);
        assert!(warnings.is_empty(), "stdlib import should not warn");
    }
}
