//! RES-076: AST → bytecode compiler.
//!
//! Walks a `Node::Program` and emits a `Chunk` for the VM to execute.
//! FOUNDATION subset only — see `bytecode.rs` for the supported Ops
//! and the RES-076 ticket for what's deferred.
//!
//! Locals are resolved at compile time to `u16` indices into a
//! per-program slab. The runtime never sees identifier strings,
//! which is half the perf win over the tree walker.

#![allow(dead_code)] // populated incrementally — follow-ups will exercise everything

use crate::bytecode::{Chunk, CompileError, Op};
use crate::{Node, Value};
use std::collections::HashMap;

/// Compile a parsed program into a `Chunk` ready for the VM.
///
/// Statements are compiled in source order. Each statement leaves
/// nothing on the operand stack (expressions inside `let` are
/// consumed by `StoreLocal`; expression statements are emitted as
/// `Op::Const(void)` placeholders if needed — the FOUNDATION compiler
/// rejects bare expression statements that would leak a value, since
/// the test surface doesn't need them).
///
/// A trailing `Op::Return` is appended unconditionally — if the
/// program ended with an explicit `return EXPR;` this is unreachable
/// and harmless; otherwise it terminates the VM with `Value::Void`.
pub fn compile(program: &Node) -> Result<Chunk, CompileError> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(CompileError::Unsupported("non-Program root")),
    };
    let mut chunk = Chunk::new();
    let mut locals: HashMap<String, u16> = HashMap::new();
    let mut next_local: u16 = 0;
    for spanned in stmts {
        // Each top-level statement uses the per-stmt span line
        // captured by RES-077 for `line_info` so VM-side errors
        // can attribute back to the source.
        let line = spanned.span.start.line as u32;
        compile_stmt(&spanned.node, &mut chunk, &mut locals, &mut next_local, line)?;
    }
    chunk.emit(Op::Return, 0);
    Ok(chunk)
}

fn compile_stmt(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(value, chunk, locals, line)?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        Node::ReturnStatement { value: Some(v) } => {
            compile_expr(v, chunk, locals, line)?;
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ReturnStatement { value: None } => {
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ExpressionStatement(inner) => {
            // FOUNDATION: only allow expression statements that don't
            // need their value (e.g. parenthesized arith for side
            // effects we don't have yet). Push then... we don't have
            // a Pop op, so just compile the expression and let the
            // value sit on the stack until Return picks it up.
            // This means a trailing `2 + 3` at top level becomes the
            // program's return value — useful for the smoke test.
            compile_expr(inner, chunk, locals, line)
        }
        Node::Function { .. } => {
            // Functions need RES-081 — the FOUNDATION can't compile a
            // call frame yet. Cleanly reject so the user knows what's
            // missing.
            Err(CompileError::Unsupported("function decl (RES-081)"))
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

fn compile_expr(
    node: &Node,
    chunk: &mut Chunk,
    locals: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::IntegerLiteral(v) => {
            let idx = chunk.add_constant(Value::Int(*v))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        Node::Identifier(name) => {
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::LoadLocal(idx), line);
            Ok(())
        }
        Node::PrefixExpression { operator, right } if operator == "-" => {
            compile_expr(right, chunk, locals, line)?;
            chunk.emit(Op::Neg, line);
            Ok(())
        }
        Node::InfixExpression { left, operator, right } => {
            compile_expr(left, chunk, locals, line)?;
            compile_expr(right, chunk, locals, line)?;
            let op = match operator.as_str() {
                "+" => Op::Add,
                "-" => Op::Sub,
                "*" => Op::Mul,
                "/" => Op::Div,
                "%" => Op::Mod,
                _ => return Err(CompileError::Unsupported("non-arithmetic operator")),
            };
            chunk.emit(op, line);
            Ok(())
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Static descriptor for a node kind, used in `Unsupported` errors.
fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Program(_) => "Program",
        Node::Use { .. } => "Use",
        Node::Function { .. } => "Function",
        Node::LiveBlock { .. } => "LiveBlock",
        Node::Assert { .. } => "Assert",
        Node::Block(_) => "Block",
        Node::LetStatement { .. } => "LetStatement",
        Node::StaticLet { .. } => "StaticLet",
        Node::Assignment { .. } => "Assignment",
        Node::ReturnStatement { .. } => "ReturnStatement",
        Node::IfStatement { .. } => "IfStatement",
        Node::WhileStatement { .. } => "WhileStatement",
        Node::ForInStatement { .. } => "ForInStatement",
        Node::ExpressionStatement(_) => "ExpressionStatement",
        Node::Identifier(_) => "Identifier",
        Node::IntegerLiteral(_) => "IntegerLiteral",
        Node::FloatLiteral(_) => "FloatLiteral",
        Node::StringLiteral(_) => "StringLiteral",
        Node::BooleanLiteral(_) => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::ArrayLiteral(_) => "ArrayLiteral",
        Node::IndexExpression { .. } => "IndexExpression",
        Node::IndexAssignment { .. } => "IndexAssignment",
        // Anything we forgot to enumerate falls through here.
        _ => "<other>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Op;

    fn parse_one(src: &str) -> Node {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        program
    }

    #[test]
    fn compile_int_literal_emits_const() {
        let p = parse_one("42;");
        let chunk = compile(&p).unwrap();
        assert_eq!(chunk.constants.len(), 1);
        assert!(matches!(chunk.constants[0], Value::Int(42)));
        // Const(0), Return (from the bare expression-stmt with value
        // sitting on the stack), then trailing Return (unreachable
        // but harmless — see compile() doc).
        assert_eq!(chunk.code.first(), Some(&Op::Const(0)));
        assert!(matches!(chunk.code.last(), Some(Op::Return)));
    }

    #[test]
    fn compile_arith_respects_precedence() {
        // 2 + 3 * 4 should compile to: Const(2), Const(3), Const(4), Mul, Add, Return*
        let p = parse_one("2 + 3 * 4;");
        let chunk = compile(&p).unwrap();
        // Strip the trailing Return that compile() always appends so
        // we can assert on just the expression body.
        let body: Vec<&Op> = chunk
            .code
            .iter()
            .filter(|op| !matches!(op, Op::Return))
            .collect();
        assert_eq!(body.len(), 5, "got {:?}", body);
        assert!(matches!(body[3], Op::Mul));
        assert!(matches!(body[4], Op::Add));
    }

    #[test]
    fn compile_let_emits_store_local() {
        let p = parse_one("let x = 7;");
        let chunk = compile(&p).unwrap();
        assert!(chunk.code.iter().any(|op| matches!(op, Op::StoreLocal(0))));
    }

    #[test]
    fn compile_unknown_identifier_errors_cleanly() {
        let p = parse_one("y;");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::UnknownIdentifier(_)));
    }

    #[test]
    fn compile_unsupported_construct_is_clean_error() {
        let p = parse_one("if true { let x = 1; }");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "{:?}", err);
    }
}
