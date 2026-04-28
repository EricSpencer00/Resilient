//! RES-324: `mod name { ... }` inline namespace blocks.
//!
//! A `mod` block groups declarations under a namespace prefix. Every
//! `fn` declared inside `mod math { ... }` is registered in the
//! environment as `"math::fn_name"`. Call sites write `math::add(1, 2)`,
//! which the parser already collapses into a flat
//! `Node::Identifier { name: "math::add" }` via the `::` token path —
//! so no extra runtime lookup machinery is needed.

use crate::{Environment, Interpreter, Node, RResult, Value};

/// Evaluate a `mod name { ... }` block.
///
/// Each `fn` in `body` is renamed to `"mod_name::fn_name"` before being
/// registered in the outer environment, making the binding visible to
/// subsequent call sites that use the `name::item` syntax.
///
/// Struct declarations inside the block are similarly prefixed.
/// Other statements (helper `let` bindings, bare expressions) are
/// evaluated in a temporary enclosed scope and do not pollute the outer
/// environment.
pub(crate) fn eval_module(
    mod_name: &str,
    body: &[Node],
    interp: &mut Interpreter,
) -> RResult<Value> {
    for node in body {
        match node {
            Node::Function { name, .. } => {
                let mut renamed = node.clone();
                if let Node::Function {
                    name: ref mut n, ..
                } = renamed
                {
                    *n = format!("{}::{}", mod_name, name);
                }
                interp.eval(&renamed)?;
            }
            Node::StructDecl { name, .. } => {
                let mut renamed = node.clone();
                if let Node::StructDecl {
                    name: ref mut n, ..
                } = renamed
                {
                    *n = format!("{}::{}", mod_name, name);
                }
                interp.eval(&renamed)?;
            }
            Node::ImplBlock { .. } => {
                // impl blocks inside modules are evaluated directly; their
                // methods are already parser-mangled with the struct name
                // and do not receive an extra namespace prefix here.
                interp.eval(node)?;
            }
            _ => {
                // For other statements evaluate them in a temporary child
                // scope so they cannot clobber outer bindings.
                let saved = interp.env.clone();
                interp.env = Environment::new_enclosed(saved.clone());
                let result = interp.eval(node);
                interp.env = saved;
                result?;
            }
        }
    }
    Ok(Value::Void)
}

/// Lightweight static pass — no-op for the MVP. Future extensions can
/// enforce unique module names and verify that `name::item` references
/// resolve to declared declarations.
pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}
