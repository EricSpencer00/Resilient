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
use std::collections::HashMap;
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

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = collect_attrs();
    if attrs.is_empty() {
        return Ok(());
    }

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
