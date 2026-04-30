//! RES-406: capability gate for the volatile MMIO intrinsics.
//!
//! Walks the program AST after typechecking and rejects any call to
//! one of the eight volatile intrinsics
//! (`volatile_read_u8/16/32/64`, `volatile_write_u8/16/32/64`) that
//! is not lexically inside an `unsafe { … }` block.
//!
//! The pass runs once per compilation unit; it doesn't change the
//! AST, only collects diagnostics. Errors are surfaced through the
//! same channel as parser errors — the driver routes them into
//! `eprintln!` with the `Borrow check`-style caret renderer.
//!
//! Why this is a separate pass (not folded into the typechecker):
//!
//! * It needs only the AST shape, not the type environment. Keeping
//!   it standalone makes it cheap to run and trivial to test.
//! * The set of "privileged builtins" is closed; growing it later
//!   (raw-pointer ops, etc.) means adding to one constant rather
//!   than threading a state flag through every typechecker arm.
//! * The error messages are highly specific ("call to
//!   `volatile_read_u32` outside an `unsafe` block — wrap in
//!   `unsafe { … }`") and benefit from being authored in one place
//!   rather than as a guard inside the call typecheck arm.

use crate::Node;
use crate::volatile::VOLATILE_INTRINSIC_NAMES;

/// Walk `program`, returning a list of human-readable diagnostics
/// for every privileged-builtin call that's not inside an `unsafe`
/// block. Empty list means clean.
pub fn check_program(program: &Node) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();
    walk(program, false, &mut errs);
    errs
}

fn walk(node: &Node, inside_unsafe: bool, errs: &mut Vec<String>) {
    match node {
        Node::UnsafeBlock { body, .. } => walk(body, true, errs),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, span, .. } = function.as_ref()
                && VOLATILE_INTRINSIC_NAMES.contains(&name.as_str())
                && !inside_unsafe
            {
                errs.push(format!(
                    "{}:{}: call to `{}` outside an `unsafe` block — wrap in `unsafe {{ ... }}`",
                    span.start.line, span.start.column, name
                ));
            }
            // Always descend into args / function to catch nested calls.
            walk(function, inside_unsafe, errs);
            for a in arguments {
                walk(a, inside_unsafe, errs);
            }
        }
        // Generic structural recursion: descend into every Node-
        // shaped child without enumerating the (large) variant set.
        // We rely on the smaller set of "container" Node variants
        // and treat everything else as a leaf.
        Node::Program(stmts) => {
            for s in stmts {
                walk(&s.node, inside_unsafe, errs);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk(s, inside_unsafe, errs);
            }
        }
        Node::ExpressionStatement { expr, .. } => walk(expr, inside_unsafe, errs),
        Node::LetStatement { value, .. } => walk(value, inside_unsafe, errs),
        Node::Assignment { value, .. } => walk(value, inside_unsafe, errs),
        Node::ReturnStatement { value: Some(v), .. } => walk(v, inside_unsafe, errs),
        Node::ReturnStatement { value: None, .. } => {}
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk(condition, inside_unsafe, errs);
            walk(consequence, inside_unsafe, errs);
            if let Some(e) = alternative {
                walk(e, inside_unsafe, errs);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk(condition, inside_unsafe, errs);
            walk(body, inside_unsafe, errs);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk(iterable, inside_unsafe, errs);
            walk(body, inside_unsafe, errs);
        }
        Node::Function { body, .. } => walk(body, inside_unsafe, errs),
        Node::PrefixExpression { right, .. } => walk(right, inside_unsafe, errs),
        Node::InfixExpression { left, right, .. } => {
            walk(left, inside_unsafe, errs);
            walk(right, inside_unsafe, errs);
        }
        Node::IndexExpression { target, index, .. } => {
            walk(target, inside_unsafe, errs);
            walk(index, inside_unsafe, errs);
        }
        Node::FieldAccess { target, .. } => walk(target, inside_unsafe, errs),
        // Other node variants are leaves for our purposes (literals,
        // identifiers, type aliases, ...). Walking past them is a
        // no-op.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::parse;

    fn errs(src: &str) -> Vec<String> {
        let (program, parse_errs) = parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {:?}", parse_errs);
        super::check_program(&program)
    }

    #[test]
    fn volatile_outside_unsafe_is_an_error() {
        let e = errs("let x = volatile_read_u32(1024);");
        assert_eq!(e.len(), 1);
        assert!(e[0].contains("volatile_read_u32"));
        assert!(e[0].contains("unsafe"));
    }

    #[test]
    fn volatile_inside_unsafe_is_clean() {
        let e = errs(
            "fn main() {\n\
             unsafe {\n\
                 let x = volatile_read_u32(0);\n\
             }\n\
         }",
        );
        assert!(e.is_empty(), "expected clean, got: {:?}", e);
    }

    #[test]
    fn nested_unsafe_inherits_capability() {
        let e = errs(
            "fn main() {\n\
             unsafe {\n\
                 if true {\n\
                     volatile_write_u8(0, 1);\n\
                 }\n\
             }\n\
         }",
        );
        assert!(e.is_empty(), "expected clean, got: {:?}", e);
    }

    #[test]
    fn unsafe_does_not_leak_to_sibling_blocks() {
        // Two top-level lets: the first is inside unsafe, the second isn't.
        let e = errs(
            "fn main() {\n\
             unsafe { volatile_write_u8(0, 1); }\n\
             let bad = volatile_read_u8(0);\n\
         }",
        );
        assert_eq!(e.len(), 1, "expected exactly one error, got: {:?}", e);
        assert!(e[0].contains("volatile_read_u8"));
    }

    #[test]
    fn all_eight_intrinsics_are_gated() {
        for name in [
            "volatile_read_u8",
            "volatile_read_u16",
            "volatile_read_u32",
            "volatile_read_u64",
            "volatile_write_u8",
            "volatile_write_u16",
            "volatile_write_u32",
            "volatile_write_u64",
        ] {
            // Use 0 / (0,0) — argument validity isn't this pass's job.
            let src = if name.starts_with("volatile_read") {
                format!("let dummy = {}(0);", name)
            } else {
                format!("let dummy = {}(0, 0);", name)
            };
            let e = super::check_program(&crate::parse(&src).0);
            assert_eq!(e.len(), 1, "expected error for `{}`, got: {:?}", name, e);
        }
    }
}
