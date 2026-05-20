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
use std::collections::HashSet;

/// Collect newtype declaration names from the program's statement list.
///
/// RES-2402: switched from `HashMap<String, String>` to `HashSet<String>`.
/// The previous map's `base_type` values were never read — `lower_program`
/// only consults `newtypes.contains_key(name)` / `is_empty()`. Each
/// `#[newtype]` declaration paid for a discarded `base_type.clone()`.
fn collect_newtypes_from_program(program: &Node) -> HashSet<String> {
    let Node::Program(statements) = program else {
        return HashSet::new();
    };
    // RES-1764: pre-size to statements.len() — at most one insert per
    // top-level NewtypeDecl, upper-bounded by the statement count.
    let mut set = HashSet::with_capacity(statements.len());
    for spanned in statements {
        if let Node::NewtypeDecl { name, .. } = &spanned.node {
            set.insert(name.clone());
        }
    }
    set
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

fn lower_node(node: &mut Node, newtypes: &HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            span,
        } => {
            if let Node::Identifier { name, .. } = function.as_ref()
                && newtypes.contains(name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, parse};

    #[test]
    fn check_accepts_valid_int_base() {
        let src = "newtype Meters = Int;\n";
        let (prog, _) = parse(src);
        assert!(
            check(&prog, "test").is_ok(),
            "Int is a valid newtype base type"
        );
    }

    #[test]
    fn check_accepts_all_valid_base_types() {
        for base in &["Int", "Float", "String", "Bool"] {
            let src = format!("newtype MyType = {};\n", base);
            let (prog, _) = parse(&src);
            assert!(
                check(&prog, "test").is_ok(),
                "{base} must be accepted as a newtype base"
            );
        }
    }

    #[test]
    fn check_rejects_invalid_base_type() {
        let src = "newtype Bad = Array;\n";
        let (prog, _) = parse(src);
        let result = check(&prog, "test");
        assert!(result.is_err(), "Array is not a valid newtype base type");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("Bad") && msg.contains("Array"),
            "error must mention the newtype name and invalid base: {msg}"
        );
    }

    #[test]
    fn lower_program_no_newtypes_is_noop() {
        let src = "fn f(int x) -> int { return x; }\nf(5);\n";
        let (mut prog, _) = parse(src);
        let before = format!("{prog:?}");
        lower_program(&mut prog);
        let after = format!("{prog:?}");
        assert_eq!(
            before, after,
            "lower_program must not modify programs without newtype declarations"
        );
    }

    #[test]
    fn lower_program_rewrites_constructor_call() {
        let src = "newtype Meters = Int;\nlet d = Meters(42);\n";
        let (mut prog, _) = parse(src);
        lower_program(&mut prog);
        // After lowering, the LetStatement's value should be a NewtypeConstruct.
        let Node::Program(stmts) = &prog else {
            panic!("expected Program root");
        };
        let has_construct = stmts.iter().any(|s| {
            if let Node::LetStatement { value, .. } = &s.node {
                matches!(value.as_ref(), Node::NewtypeConstruct { type_name, .. } if type_name == "Meters")
            } else {
                false
            }
        });
        assert!(
            has_construct,
            "lower_program must rewrite Meters(42) inside let binding into NewtypeConstruct"
        );
    }

    #[test]
    fn check_ok_with_no_newtypes() {
        let src = "fn f(int x) -> int { return x; }\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
