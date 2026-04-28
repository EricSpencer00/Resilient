//! RES-319: newtype declarations — `newtype Meters = Float;`.
//!
//! A newtype creates a fresh nominal type distinct from its base. The
//! constructor `Meters(5.0)` wraps a `Float` and produces a `Meters`.
//! Arithmetic operators are inherited by carrying the operation through
//! to the inner value and re-wrapping. Mixing different newtypes of the
//! same base (e.g. adding `Meters` to `Seconds`) is a type error caught
//! in this post-parse pass before `eval` runs.
//!
//! # Architecture
//!
//! The hot `eval` path is intentionally untouched. All newtype logic runs
//! in two distinct phases:
//!
//! 1. **`lower_program`** (post-parse) — rewrites every `CallExpression`
//!    whose callee name matches a declared newtype into a
//!    `Node::NewtypeConstruct` node. This must happen before `eval`.
//!
//! 2. **`check`** (typechecker extension pass) — validates that all
//!    declared base types are known primitives.
//!
//! This two-phase layout avoids any overhead inside `eval(InfixExpression)`
//! or other hot arms — critical because the fib(10) recursion test showed
//! that even a cheap `if` check there can overflow the default 2 MiB test
//! thread stack.

use crate::Node;
use std::collections::HashMap;

/// Collect all newtype declarations from the program's statement list into
/// a `name → base_type` map.
fn collect_newtypes_from_program(program: &Node) -> HashMap<String, String> {
    let Node::Program(statements) = program else {
        return HashMap::new();
    };
    let mut map = HashMap::new();
    for spanned in statements {
        if let Node::NewtypeDecl {
            name, base_type, ..
        } = &spanned.node
        {
            map.insert(name.clone(), base_type.clone());
        }
    }
    map
}

/// Post-parse lowering: rewrite every `Foo(expr)` `CallExpression` node
/// into a `Node::NewtypeConstruct` node when `Foo` is a declared newtype.
///
/// Accepts the root `Node::Program` node. No-ops on anything else. The
/// function signature mirrors the pattern used by `try_catch`, `type_aliases`,
/// and other post-parse passes so the `<EXTENSION_PASSES>` block in `main.rs`
/// can call it uniformly.
pub fn lower_program(program: &mut Node) {
    // Collect before mutating to avoid borrow-checker conflict.
    let newtypes = collect_newtypes_from_program(program);
    if newtypes.is_empty() {
        return;
    }
    let Node::Program(statements) = program else {
        return;
    };
    for spanned in statements.iter_mut() {
        lower_node(&mut spanned.node, &newtypes);
    }
}

fn lower_node(node: &mut Node, newtypes: &HashMap<String, String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && newtypes.contains_key(name)
                && arguments.len() == 1
            {
                // Lower the inner argument first so nested constructors work.
                lower_node(&mut arguments[0], newtypes);
                *node = Node::NewtypeConstruct {
                    type_name: name.clone(),
                    value: Box::new(arguments[0].clone()),
                    span: *span,
                };
                return;
            }
            // Not a newtype call — recurse into arguments.
            for arg in arguments.iter_mut() {
                lower_node(arg, newtypes);
            }
        }
        Node::Function { body, .. } => {
            lower_node(body, newtypes);
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts.iter_mut() {
                lower_node(stmt, newtypes);
            }
        }
        Node::LetStatement { value, .. } => {
            lower_node(value, newtypes);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            lower_node(v, newtypes);
        }
        Node::ExpressionStatement { expr, .. } => {
            lower_node(expr, newtypes);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            lower_node(condition, newtypes);
            lower_node(consequence, newtypes);
            if let Some(alt) = alternative {
                lower_node(alt, newtypes);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            lower_node(condition, newtypes);
            lower_node(body, newtypes);
        }
        Node::ForInStatement { iterable, body, .. } => {
            lower_node(iterable, newtypes);
            lower_node(body, newtypes);
        }
        Node::InfixExpression { left, right, .. } => {
            lower_node(left, newtypes);
            lower_node(right, newtypes);
        }
        Node::PrefixExpression { right, .. } => {
            lower_node(right, newtypes);
        }
        Node::NewtypeConstruct { value, .. } => {
            lower_node(value, newtypes);
        }
        // All other nodes carry no sub-expressions that can contain a
        // newtype constructor, or are leaf nodes.
        _ => {}
    }
}

/// Typechecker extension pass — validates all newtype declarations.
///
/// Accepts the root `Node::Program` node (matching the signature convention
/// used by `try_catch::check`, `type_aliases::check`, etc.).
pub fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(statements) = program else {
        return Ok(());
    };
    const VALID_BASES: &[&str] = &["Int", "Float", "String", "Bool"];
    for spanned in statements {
        if let Node::NewtypeDecl {
            name, base_type, ..
        } = &spanned.node
            && !VALID_BASES.contains(&base_type.as_str())
        {
            return Err(format!(
                "newtype `{}` has unknown base type `{}` — expected one of: Int, Float, String, Bool",
                name, base_type
            ));
        }
    }
    Ok(())
}
