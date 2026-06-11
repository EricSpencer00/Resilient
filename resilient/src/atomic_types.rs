//! Feature 33/50 — Atomic Types.
//!
//! `#[atomic]` on a `static let` binding marks it as a lock-free
//! shared cell. The runtime backs it by a Rust `AtomicI64` and
//! exposes ordering-aware accessor builtins:
//!
//! * `atomic_load(name) -> int`
//! * `atomic_store(name, value)`
//! * `atomic_fetch_add(name, delta) -> int`
//!
//! The first slice ships the registry of atomic names so the runtime
//! and typechecker can validate usage.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::sync::atomic::{AtomicI64, Ordering};

#[derive(Debug, Default)]
struct AtomicRegistry {
    cells: HashMap<String, AtomicI64>,
}

static REGISTRY: RwLock<Option<AtomicRegistry>> = RwLock::new(None);

pub fn collect_names() -> Vec<String> {
    crate::feature_attrs::find_kind("atomic")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

fn collect_attrs() -> Vec<(String, crate::feature_attrs::AttrRecord)> {
    crate::feature_attrs::find_kind("atomic")
}

// RES-1406: removed `fn ensure()` — its sole caller was `declare`,
// and `declare`'s own `g.get_or_insert_with(AtomicRegistry::default)`
// already creates the registry on first use. `ensure()` was acquiring
// the `RwLock` write guard purely to check / initialise the Option,
// then `declare` immediately re-acquired the same write guard to do
// the actual insert. One acquire is enough.

pub fn declare(name: &str, initial: i64) {
    declare_owned(name.to_string(), initial);
}

/// RES-2206: inner helper that consumes an owned `String` instead of
/// cloning from a borrow. The `check` path below collects owned
/// names from `feature_attrs::find_kind("atomic")` and immediately
/// throws them away — moving each name straight into the registry
/// avoids the `name.to_string()` clone that the previous shape paid
/// per `#[atomic]` attribute on top of the `collect_names` owned
/// strings the attribute walker had already produced.
fn declare_owned(name: String, initial: i64) {
    if let Ok(mut g) = REGISTRY.write() {
        let r = g.get_or_insert_with(AtomicRegistry::default);
        r.cells.insert(name, AtomicI64::new(initial));
    }
}

pub fn load(name: &str) -> Option<i64> {
    REGISTRY.read().ok().and_then(|g| {
        g.as_ref()
            .and_then(|r| r.cells.get(name).map(|a| a.load(Ordering::SeqCst)))
    })
}

pub fn store(name: &str, value: i64) -> bool {
    if let Ok(g) = REGISTRY.read() {
        if let Some(r) = g.as_ref() {
            if let Some(a) = r.cells.get(name) {
                a.store(value, Ordering::SeqCst);
                return true;
            }
        }
    }
    false
}

pub fn fetch_add(name: &str, delta: i64) -> Option<i64> {
    REGISTRY.read().ok().and_then(|g| {
        g.as_ref().and_then(|r| {
            r.cells
                .get(name)
                .map(|a| a.fetch_add(delta, Ordering::SeqCst))
        })
    })
}

#[derive(Clone, Copy)]
enum AtomicTarget<'a> {
    StaticLet { value: &'a Node, span: Span },
    Other { kind: &'static str, span: Span },
}

fn find_atomic_target<'a>(node: &'a Node, name: &str) -> Option<AtomicTarget<'a>> {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                if let Some(found) = find_atomic_target(&stmt.node, name) {
                    return Some(found);
                }
            }
            None
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                if let Some(found) = find_atomic_target(stmt, name) {
                    return Some(found);
                }
            }
            None
        }
        Node::StaticLet {
            name: decl_name,
            value,
            span,
        } if decl_name == name => Some(AtomicTarget::StaticLet {
            value: value.as_ref(),
            span: *span,
        }),
        Node::LetStatement {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "`let` binding",
            span: *span,
        }),
        Node::Function {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "function",
            span: *span,
        }),
        Node::StructDecl {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "struct",
            span: *span,
        }),
        Node::TypeAlias {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "type alias",
            span: *span,
        }),
        Node::NewtypeDecl {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "newtype",
            span: *span,
        }),
        Node::EnumDecl {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "enum",
            span: *span,
        }),
        Node::TraitDecl {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "trait",
            span: *span,
        }),
        Node::ActorDecl {
            name: decl_name,
            span,
            ..
        } if decl_name == name => Some(AtomicTarget::Other {
            kind: "actor",
            span: *span,
        }),
        Node::Function { body, .. } => find_atomic_target(body, name),
        _ => None,
    }
}

fn static_integer_value(node: &Node) -> Option<i64> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator: "-",
            right,
            ..
        } => match right.as_ref() {
            Node::IntegerLiteral { value, .. } => value.checked_neg(),
            _ => None,
        },
        Node::PrefixExpression {
            operator: "+",
            right,
            ..
        } => match right.as_ref() {
            Node::IntegerLiteral { value, .. } => Some(*value),
            _ => None,
        },
        _ => None,
    }
}

fn diagnostic(source_path: &str, span: Span, message: &str) -> String {
    format!(
        "{}:{}:{}: error: {}",
        source_path, span.start.line, span.start.column, message
    )
}

fn span_of(node: &Node) -> Span {
    match node {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::StringInternLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::CharLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. }
        | Node::FunctionLiteral { span, .. }
        | Node::Match { span, .. }
        | Node::StructDecl { span, .. }
        | Node::LetDestructureStruct { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::Slice { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::MapLiteral { span, .. }
        | Node::SetLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::TraitDecl { span, .. }
        | Node::TypeAlias { span, .. }
        | Node::RegionDecl { span, .. }
        | Node::Actor { span, .. }
        | Node::ActorDecl { span, .. }
        | Node::ClusterDecl { span, .. }
        | Node::TryCatch { span, .. }
        | Node::Quantifier { span, .. }
        | Node::InvariantStatement { span, .. }
        | Node::Range { span, .. }
        | Node::NamedArg { span, .. }
        | Node::InterpolatedString { span, .. }
        | Node::ModuleDecl { span, .. }
        | Node::NewtypeDecl { span, .. }
        | Node::NewtypeConstruct { span, .. }
        | Node::SupervisorDecl { span, .. }
        | Node::TupleLiteral { span, .. }
        | Node::TupleIndex { span, .. }
        | Node::LetTupleDestructure { span, .. }
        | Node::UnsafeBlock { span, .. }
        | Node::EnumDecl { span, .. }
        | Node::RegionParam { span, .. }
        | Node::BlanketImpl { span, .. }
        | Node::StaticAssert { span, .. }
        | Node::BenchBlock { span, .. }
        | Node::Use { span, .. }
        | Node::Extern { span, .. }
        | Node::Function { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::DurationLiteral { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. }
        | Node::Block { span, .. }
        | Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Const { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::Break { span, .. }
        | Node::BreakWith { span, .. }
        | Node::Continue { span, .. }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. }
        | Node::DeferStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. } => *span,
        Node::Program(_) => Span::default(),
    }
}

fn arg_kind(node: &Node) -> &'static str {
    match node {
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. } => "string literal",
        Node::IntegerLiteral { .. } | Node::FloatLiteral { .. } => "number literal",
        Node::PrefixExpression {
            operator: "+" | "-",
            right,
            ..
        } if matches!(
            right.as_ref(),
            Node::IntegerLiteral { .. } | Node::FloatLiteral { .. }
        ) =>
        {
            "number literal"
        }
        Node::ArrayLiteral { .. } => "list literal",
        Node::StructLiteral { .. } => "struct literal",
        Node::BooleanLiteral { .. } => "boolean literal",
        Node::CharLiteral { .. } => "char literal",
        Node::BytesLiteral { .. } => "bytes literal",
        Node::MapLiteral { .. } => "map literal",
        Node::SetLiteral { .. } => "set literal",
        Node::CallExpression { .. } => "call expression",
        _ => "expression",
    }
}

fn atomic_call_name(function: &Node) -> Option<&'static str> {
    match function {
        Node::Identifier { name, .. } => match name.as_str() {
            "atomic_load" => Some("atomic_load"),
            "atomic_store" => Some("atomic_store"),
            "atomic_fetch_add" => Some("atomic_fetch_add"),
            _ => None,
        },
        _ => None,
    }
}

fn validate_atomic_target(
    source_path: &str,
    op: &str,
    target: &Node,
    atomic_names: &HashSet<String>,
) -> Result<(), String> {
    match target {
        Node::Identifier { name, span } => {
            if atomic_names.contains(name) {
                Ok(())
            } else {
                Err(diagnostic(
                    source_path,
                    *span,
                    &format!("{} target `{}` is not declared #[atomic]", op, name),
                ))
            }
        }
        other => Err(diagnostic(
            source_path,
            span_of(other),
            &format!(
                "{} target must be an atomic identifier, got {}",
                op,
                arg_kind(other)
            ),
        )),
    }
}

fn validate_atomic_integer_arg(source_path: &str, op: &str, arg: &Node) -> Result<(), String> {
    match arg {
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. }
        | Node::ArrayLiteral { .. }
        | Node::StructLiteral { .. }
        | Node::MapLiteral { .. }
        | Node::SetLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. } => Err(diagnostic(
            source_path,
            span_of(arg),
            &format!(
                "{} value must be an integer expression, got {}",
                op,
                arg_kind(arg)
            ),
        )),
        _ => Ok(()),
    }
}

fn validate_atomic_call(
    source_path: &str,
    op: &str,
    arguments: &[Node],
    call_span: Span,
    atomic_names: &HashSet<String>,
) -> Result<(), String> {
    let expected = if op == "atomic_load" { 1 } else { 2 };
    if arguments.len() != expected {
        return Err(diagnostic(
            source_path,
            call_span,
            &format!(
                "{} expects {} arguments, got {}",
                op,
                expected,
                arguments.len()
            ),
        ));
    }
    validate_atomic_target(source_path, op, &arguments[0], atomic_names)?;
    if expected == 2 {
        validate_atomic_integer_arg(source_path, op, &arguments[1])?;
    }
    Ok(())
}

fn check_atomic_call_sites(
    node: &Node,
    source_path: &str,
    atomic_names: &HashSet<String>,
) -> Result<(), String> {
    match node {
        Node::Program(stmts) => {
            for stmt in stmts {
                check_atomic_call_sites(&stmt.node, source_path, atomic_names)?;
            }
        }
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Some(op) = atomic_call_name(function) {
                validate_atomic_call(source_path, op, arguments, *span, atomic_names)?;
            }
            check_atomic_call_sites(function, source_path, atomic_names)?;
            for arg in arguments {
                check_atomic_call_sites(arg, source_path, atomic_names)?;
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                check_atomic_call_sites(stmt, source_path, atomic_names)?;
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. }
        | Node::BreakWith { value, .. }
        | Node::DeferStatement { expr: value, .. }
        | Node::ExpressionStatement { expr: value, .. }
        | Node::TryExpression { expr: value, .. }
        | Node::InvariantStatement { expr: value, .. }
        | Node::NamedArg { value, .. }
        | Node::NewtypeConstruct { value, .. }
        | Node::BenchBlock { body: value, .. }
        | Node::UnsafeBlock { body: value, .. } => {
            check_atomic_call_sites(value, source_path, atomic_names)?;
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(value) = value {
                check_atomic_call_sites(value, source_path, atomic_names)?;
            }
        }
        Node::Assert {
            condition, message, ..
        }
        | Node::Assume {
            condition, message, ..
        } => {
            check_atomic_call_sites(condition, source_path, atomic_names)?;
            if let Some(message) = message {
                check_atomic_call_sites(message, source_path, atomic_names)?;
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_atomic_call_sites(condition, source_path, atomic_names)?;
            check_atomic_call_sites(consequence, source_path, atomic_names)?;
            if let Some(alternative) = alternative {
                check_atomic_call_sites(alternative, source_path, atomic_names)?;
            }
        }
        Node::WhileStatement {
            condition,
            body,
            invariants,
            ..
        } => {
            check_atomic_call_sites(condition, source_path, atomic_names)?;
            for invariant in invariants {
                check_atomic_call_sites(invariant, source_path, atomic_names)?;
            }
            check_atomic_call_sites(body, source_path, atomic_names)?;
        }
        Node::ForInStatement {
            iterable,
            body,
            invariants,
            ..
        } => {
            check_atomic_call_sites(iterable, source_path, atomic_names)?;
            for invariant in invariants {
                check_atomic_call_sites(invariant, source_path, atomic_names)?;
            }
            check_atomic_call_sites(body, source_path, atomic_names)?;
        }
        Node::PrefixExpression { right, .. } => {
            check_atomic_call_sites(right, source_path, atomic_names)?;
        }
        Node::InfixExpression { left, right, .. } => {
            check_atomic_call_sites(left, source_path, atomic_names)?;
            check_atomic_call_sites(right, source_path, atomic_names)?;
        }
        Node::OptionalChain { object, access, .. } => {
            check_atomic_call_sites(object, source_path, atomic_names)?;
            if let crate::ChainAccess::Method(_, arguments) = access {
                for arg in arguments {
                    check_atomic_call_sites(arg, source_path, atomic_names)?;
                }
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
                check_atomic_call_sites(default, source_path, atomic_names)?;
            }
            for expr in requires.iter().chain(ensures.iter()) {
                check_atomic_call_sites(expr, source_path, atomic_names)?;
            }
            if let Some(recovers_to) = recovers_to {
                check_atomic_call_sites(recovers_to, source_path, atomic_names)?;
            }
            check_atomic_call_sites(body, source_path, atomic_names)?;
        }
        Node::FunctionLiteral {
            body,
            requires,
            ensures,
            recovers_to,
            ..
        } => {
            for expr in requires.iter().chain(ensures.iter()) {
                check_atomic_call_sites(expr, source_path, atomic_names)?;
            }
            if let Some(recovers_to) = recovers_to {
                check_atomic_call_sites(recovers_to, source_path, atomic_names)?;
            }
            check_atomic_call_sites(body, source_path, atomic_names)?;
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            check_atomic_call_sites(scrutinee, source_path, atomic_names)?;
            for (_, guard, body) in arms {
                if let Some(guard) = guard {
                    check_atomic_call_sites(guard, source_path, atomic_names)?;
                }
                check_atomic_call_sites(body, source_path, atomic_names)?;
            }
        }
        Node::LetDestructureStruct { value, .. }
        | Node::FieldAccess { target: value, .. }
        | Node::TupleIndex { tuple: value, .. }
        | Node::LetTupleDestructure { value, .. } => {
            check_atomic_call_sites(value, source_path, atomic_names)?;
        }
        Node::StructLiteral { fields, base, .. } => {
            if let Some(base) = base {
                check_atomic_call_sites(base, source_path, atomic_names)?;
            }
            for (_, value) in fields {
                check_atomic_call_sites(value, source_path, atomic_names)?;
            }
        }
        Node::FieldAssignment { target, value, .. } => {
            check_atomic_call_sites(target, source_path, atomic_names)?;
            check_atomic_call_sites(value, source_path, atomic_names)?;
        }
        Node::ArrayLiteral { items, .. }
        | Node::SetLiteral { items, .. }
        | Node::TupleLiteral { items, .. } => {
            for item in items {
                check_atomic_call_sites(item, source_path, atomic_names)?;
            }
        }
        Node::IndexExpression { target, index, .. } => {
            check_atomic_call_sites(target, source_path, atomic_names)?;
            check_atomic_call_sites(index, source_path, atomic_names)?;
        }
        Node::Slice { target, lo, hi, .. } => {
            check_atomic_call_sites(target, source_path, atomic_names)?;
            if let Some(lo) = lo {
                check_atomic_call_sites(lo, source_path, atomic_names)?;
            }
            if let Some(hi) = hi {
                check_atomic_call_sites(hi, source_path, atomic_names)?;
            }
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            check_atomic_call_sites(target, source_path, atomic_names)?;
            check_atomic_call_sites(index, source_path, atomic_names)?;
            check_atomic_call_sites(value, source_path, atomic_names)?;
        }
        Node::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                check_atomic_call_sites(key, source_path, atomic_names)?;
                check_atomic_call_sites(value, source_path, atomic_names)?;
            }
        }
        Node::ImplBlock { methods, .. } | Node::BlanketImpl { methods, .. } => {
            for method in methods {
                check_atomic_call_sites(method, source_path, atomic_names)?;
            }
        }
        Node::Actor {
            state_init,
            concurrent_ensures,
            handlers,
            ..
        } => {
            check_atomic_call_sites(state_init, source_path, atomic_names)?;
            for expr in concurrent_ensures {
                check_atomic_call_sites(expr, source_path, atomic_names)?;
            }
            for handler in handlers {
                check_atomic_call_sites(&handler.body, source_path, atomic_names)?;
                for expr in &handler.ensures {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
        }
        Node::ActorDecl {
            state_fields,
            always_clauses,
            eventually_clauses,
            receive_handlers,
            handlers,
            ..
        } => {
            for (_, _, init) in state_fields {
                check_atomic_call_sites(init, source_path, atomic_names)?;
            }
            for expr in always_clauses {
                check_atomic_call_sites(expr, source_path, atomic_names)?;
            }
            for clause in eventually_clauses {
                check_atomic_call_sites(&clause.post, source_path, atomic_names)?;
            }
            for handler in receive_handlers {
                check_atomic_call_sites(&handler.body, source_path, atomic_names)?;
                for expr in handler.requires.iter().chain(handler.ensures.iter()) {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
            for handler in handlers {
                check_atomic_call_sites(&handler.body, source_path, atomic_names)?;
                for expr in &handler.ensures {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
        }
        Node::ClusterDecl { invariants, .. } => {
            for invariant in invariants {
                check_atomic_call_sites(invariant, source_path, atomic_names)?;
            }
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                check_atomic_call_sites(stmt, source_path, atomic_names)?;
            }
            for (_, stmts) in handlers {
                for stmt in stmts {
                    check_atomic_call_sites(stmt, source_path, atomic_names)?;
                }
            }
        }
        Node::Quantifier { range, body, .. } => {
            match range {
                crate::quantifiers::QuantRange::Range { lo, hi } => {
                    check_atomic_call_sites(lo, source_path, atomic_names)?;
                    check_atomic_call_sites(hi, source_path, atomic_names)?;
                }
                crate::quantifiers::QuantRange::Iterable(expr) => {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
            check_atomic_call_sites(body, source_path, atomic_names)?;
        }
        Node::Range { lo, hi, .. } => {
            check_atomic_call_sites(lo, source_path, atomic_names)?;
            check_atomic_call_sites(hi, source_path, atomic_names)?;
        }
        Node::InterpolatedString { parts, .. } => {
            for part in parts {
                if let crate::string_interp::StringPart::Expr(expr) = part {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
        }
        Node::ModuleDecl { body, .. } => {
            for stmt in body {
                check_atomic_call_sites(stmt, source_path, atomic_names)?;
            }
        }
        Node::StaticAssert { condition, .. } => {
            check_atomic_call_sites(condition, source_path, atomic_names)?;
        }
        Node::LiveBlock {
            body,
            invariants,
            timeout,
            ..
        } => {
            check_atomic_call_sites(body, source_path, atomic_names)?;
            for invariant in invariants {
                check_atomic_call_sites(invariant, source_path, atomic_names)?;
            }
            if let Some(timeout) = timeout {
                check_atomic_call_sites(timeout, source_path, atomic_names)?;
            }
        }
        Node::Extern { decls, .. } => {
            for decl in decls {
                for expr in &decl.requires {
                    check_atomic_call_sites(expr, source_path, atomic_names)?;
                }
            }
        }
        Node::Identifier { .. }
        | Node::IntegerLiteral { .. }
        | Node::FloatLiteral { .. }
        | Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::BytesLiteral { .. }
        | Node::CharLiteral { .. }
        | Node::BooleanLiteral { .. }
        | Node::StructDecl { .. }
        | Node::TraitDecl { .. }
        | Node::TypeAlias { .. }
        | Node::RegionDecl { .. }
        | Node::NewtypeDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::EnumDecl { .. }
        | Node::RegionParam { .. }
        | Node::Use { .. }
        | Node::DurationLiteral { .. }
        | Node::Break { .. }
        | Node::Continue { .. }
        | Node::BreakLabel { .. }
        | Node::ContinueLabel { .. } => {}
    }
    Ok(())
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = collect_attrs();
    let atomic_names: HashSet<String> = attrs.iter().map(|(name, _)| name.clone()).collect();

    // RES-2206: move each owned `String` straight into the registry
    // via `declare_owned`. The previous `declare(&n, 0)` form borrowed
    // `n` into `declare`, which then called `name.to_string()` —
    // paying a fresh allocation per `#[atomic]` name on top of the
    // one `collect_names` already produced.
    for (name, rec) in attrs {
        let target = find_atomic_target(program, name.as_str());
        let span = match target {
            Some(AtomicTarget::StaticLet { span, .. } | AtomicTarget::Other { span, .. }) => span,
            None => Span::default(),
        };
        if !rec.args.trim().is_empty() {
            return Err(diagnostic(
                source_path,
                span,
                &format!(
                    "#[atomic] on `{}` does not accept arguments; use bare #[atomic]",
                    name
                ),
            ));
        }
        match target {
            Some(AtomicTarget::StaticLet { value, span }) => {
                let Some(initial) = static_integer_value(value) else {
                    return Err(diagnostic(
                        source_path,
                        span,
                        &format!(
                            "atomic type `{}` must be initialized with an integer literal",
                            name
                        ),
                    ));
                };
                declare_owned(name, initial);
            }
            Some(AtomicTarget::Other { kind, span }) => {
                return Err(diagnostic(
                    source_path,
                    span,
                    &format!(
                        "atomic type `{}` must be declared as `static let`, found {}",
                        name, kind
                    ),
                ));
            }
            None => {
                return Err(diagnostic(
                    source_path,
                    span,
                    &format!("atomic type `{}` is missing a matching declaration", name),
                ));
            }
        }
    }
    check_atomic_call_sites(program, source_path, &atomic_names)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_add_is_atomic() {
        declare("counter", 0);
        let prev = fetch_add("counter", 5);
        assert_eq!(prev, Some(0));
        let prev = fetch_add("counter", 3);
        assert_eq!(prev, Some(5));
        assert_eq!(load("counter"), Some(8));
    }

    #[test]
    fn store_overwrites() {
        declare("flag", 0);
        store("flag", 42);
        assert_eq!(load("flag"), Some(42));
    }
}
