//! RES-076 + RES-081: AST → bytecode compiler.
//!
//! Walks a `Node::Program` and emits a `Program { main, functions }`
//! for the VM to execute. Supports the subset covered by RES-076
//! (int arithmetic, let bindings, identifiers, return) plus RES-081
//! (top-level function declarations + calls).
//!
//! Locals are resolved at compile time to `u16` frame-relative
//! indices; the runtime never sees identifier strings. That's half
//! the perf win over the tree walker.

#![allow(dead_code)]

use crate::bytecode::{Chunk, CompileError, Function, Op, Program};
use crate::{Node, Value};
use std::collections::HashMap;

/// Compile a parsed program into a bytecode `Program` ready for the VM.
///
/// Steps:
/// 1. Pre-pass: find every top-level `fn` and index it by name so
///    call sites can refer to it regardless of source order (mirrors
///    the tree-walker's function-hoist in `eval_program`).
/// 2. Compile each function body into its own `Chunk`.
/// 3. Compile the remaining top-level statements into `main`.
///
/// A trailing `Op::Return` is appended to `main` unconditionally —
/// if the program ended with an explicit `return EXPR;` this is
/// unreachable and harmless; otherwise it terminates the VM with
/// `Value::Void`.
pub fn compile(program: &Node) -> Result<Program, CompileError> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(CompileError::Unsupported("non-Program root")),
    };

    // Pre-pass: function name → index in the `functions` table.
    let mut fn_index: HashMap<String, u16> = HashMap::new();
    let mut next_fn_idx: u16 = 0;
    for spanned in stmts {
        if let Node::Function { name, parameters, .. } = &spanned.node {
            if parameters.len() > u8::MAX as usize {
                return Err(CompileError::Unsupported("fn with >255 params"));
            }
            if next_fn_idx == u16::MAX {
                return Err(CompileError::Unsupported("program has > 65535 functions"));
            }
            fn_index.insert(name.clone(), next_fn_idx);
            next_fn_idx += 1;
        }
    }

    // Pass 2: compile each function body in declaration order.
    let mut functions: Vec<Function> = Vec::new();
    for spanned in stmts {
        if let Node::Function { name, parameters, body, .. } = &spanned.node {
            let arity = parameters.len() as u8;
            let mut chunk = Chunk::new();
            // Parameters occupy locals 0..arity. Map each param name
            // to its slot; additional `let` bindings in the body bump
            // `next_local` from there.
            let mut locals: HashMap<String, u16> = HashMap::new();
            let mut next_local: u16 = 0;
            for (_type_name, pname) in parameters {
                locals.insert(pname.clone(), next_local);
                next_local += 1;
            }
            // Function bodies are `Node::Block(stmts)` today. Walk
            // the inner statements; emit a trailing ReturnFromCall so
            // a body that fell through produces Void to the caller.
            let inner = match body.as_ref() {
                Node::Block(b) => b,
                single => std::slice::from_ref(single),
            };
            for stmt in inner {
                let line = spanned.span.start.line as u32;
                compile_stmt_in_fn(
                    stmt,
                    &mut chunk,
                    &mut locals,
                    &mut next_local,
                    &fn_index,
                    line,
                )?;
            }
            chunk.emit(Op::ReturnFromCall, 0);
            functions.push(Function {
                name: name.clone(),
                arity,
                chunk,
                local_count: next_local,
            });
        }
    }

    // Pass 3: compile the remaining top-level statements into `main`.
    let mut main = Chunk::new();
    let mut main_locals: HashMap<String, u16> = HashMap::new();
    let mut main_next_local: u16 = 0;
    for spanned in stmts {
        // Skip fn decls — they were compiled in pass 2.
        if matches!(spanned.node, Node::Function { .. }) {
            continue;
        }
        let line = spanned.span.start.line as u32;
        compile_stmt(
            &spanned.node,
            &mut main,
            &mut main_locals,
            &mut main_next_local,
            &fn_index,
            line,
        )?;
    }
    main.emit(Op::Return, 0);

    Ok(Program { main, functions })
}

/// Compile a top-level (main-chunk) statement. Bare expression
/// statements leak their value onto the operand stack, which `Return`
/// picks up as the program result — useful for the RES-076 smoke
/// test that parses `2 + 3 * 4;`.
fn compile_stmt(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, line)?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(v, chunk, locals, fn_index, line)?;
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ExpressionStatement(inner) => {
            compile_expr(inner, chunk, locals, fn_index, line)
        }
        Node::IfStatement { .. } | Node::WhileStatement { .. } | Node::Block(_) => {
            compile_control_flow(node, chunk, locals, next_local, fn_index, line)
        }
        Node::Assignment { name, value, .. } => {
            // RES-083: re-bind an existing local. Compile the RHS,
            // StoreLocal to the known slot. Unknown name is an error.
            compile_expr(value, chunk, locals, fn_index, line)?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        Node::Function { .. } => {
            // Top-level fn decl already handled in pass 2. Skipping
            // here would be a no-op, but we should never see one —
            // the caller filters them out before calling us.
            Err(CompileError::Unsupported("nested function decl"))
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-083: compile if/while/block statements that share the same
/// locals environment as the enclosing scope. `Block` is flattened:
/// its inner statements are compiled inline (no new scope frame yet
/// — matches the tree walker's semantics).
fn compile_control_flow(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::Block(stmts) => {
            for s in stmts {
                compile_stmt(s, chunk, locals, next_local, fn_index, line)?;
            }
            Ok(())
        }
        Node::IfStatement { condition, consequence, alternative, .. } => {
            // cond
            compile_expr(condition, chunk, locals, fn_index, line)?;
            // JumpIfFalse to else-or-end (placeholder 0 offset)
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // consequence
            compile_stmt(consequence, chunk, locals, next_local, fn_index, line)?;
            if let Some(alt) = alternative {
                // Unconditional jump past the else branch
                let jmp_end = chunk.emit(Op::Jump(0), line);
                // JumpIfFalse lands here (start of else)
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt(alt, chunk, locals, next_local, fn_index, line)?;
                // And the skip-over-else lands here (end)
                let end = chunk.code.len();
                chunk.patch_jump(jmp_end, end)?;
            } else {
                // No else — JumpIfFalse lands after the consequence.
                let end = chunk.code.len();
                chunk.patch_jump(jif, end)?;
            }
            Ok(())
        }
        Node::WhileStatement { condition, body, .. } => {
            let loop_start = chunk.code.len();
            compile_expr(condition, chunk, locals, fn_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt(body, chunk, locals, next_local, fn_index, line)?;
            // Unconditional loop back to cond
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            // JumpIfFalse lands after the loop
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            Ok(())
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Compile a statement inside a `fn` body. Same as `compile_stmt`
/// except `return EXPR;` emits `ReturnFromCall` instead of `Return`
/// — a bare `return` at program scope halts the VM; one inside a
/// function returns to the caller.
fn compile_stmt_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, line)?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(v, chunk, locals, fn_index, line)?;
            chunk.emit(Op::ReturnFromCall, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            // `return;` inside a fn body returns Void — push a Void
            // constant so ReturnFromCall has something to transfer.
            let idx = chunk.add_constant(Value::Void)?;
            chunk.emit(Op::Const(idx), line);
            chunk.emit(Op::ReturnFromCall, line);
            Ok(())
        }
        Node::ExpressionStatement(inner) => {
            compile_expr(inner, chunk, locals, fn_index, line)
        }
        Node::IfStatement { .. } | Node::WhileStatement { .. } | Node::Block(_) => {
            compile_control_flow_in_fn(node, chunk, locals, next_local, fn_index, line)
        }
        Node::Assignment { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, line)?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Same as `compile_control_flow` but routes nested statements
/// through `compile_stmt_in_fn` so `return` inside a branch emits
/// `ReturnFromCall`. This is the version used by function bodies.
fn compile_control_flow_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::Block(stmts) => {
            for s in stmts {
                compile_stmt_in_fn(s, chunk, locals, next_local, fn_index, line)?;
            }
            Ok(())
        }
        Node::IfStatement { condition, consequence, alternative, .. } => {
            compile_expr(condition, chunk, locals, fn_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(consequence, chunk, locals, next_local, fn_index, line)?;
            if let Some(alt) = alternative {
                let jmp_end = chunk.emit(Op::Jump(0), line);
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt_in_fn(alt, chunk, locals, next_local, fn_index, line)?;
                let end = chunk.code.len();
                chunk.patch_jump(jmp_end, end)?;
            } else {
                let end = chunk.code.len();
                chunk.patch_jump(jif, end)?;
            }
            Ok(())
        }
        Node::WhileStatement { condition, body, .. } => {
            let loop_start = chunk.code.len();
            compile_expr(condition, chunk, locals, fn_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(body, chunk, locals, next_local, fn_index, line)?;
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            Ok(())
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

fn compile_expr(
    node: &Node,
    chunk: &mut Chunk,
    locals: &HashMap<String, u16>,
    fn_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::IntegerLiteral { value: v, .. } => {
            let idx = chunk.add_constant(Value::Int(*v))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-083: boolean literals.
        Node::BooleanLiteral { value: b, .. } => {
            let idx = chunk.add_constant(Value::Bool(*b))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        Node::Identifier { name, .. } => {
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::LoadLocal(idx), line);
            Ok(())
        }
        Node::PrefixExpression { operator, right } if operator == "-" => {
            compile_expr(right, chunk, locals, fn_index, line)?;
            chunk.emit(Op::Neg, line);
            Ok(())
        }
        // RES-083: logical negation.
        Node::PrefixExpression { operator, right } if operator == "!" => {
            compile_expr(right, chunk, locals, fn_index, line)?;
            chunk.emit(Op::Not, line);
            Ok(())
        }
        // RES-083: short-circuit && desugars to `if lhs { rhs } else { false }`.
        Node::InfixExpression { left, operator, right } if operator == "&&" => {
            compile_expr(left, chunk, locals, fn_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_expr(right, chunk, locals, fn_index, line)?;
            let jmp_end = chunk.emit(Op::Jump(0), line);
            // false branch
            let false_target = chunk.code.len();
            chunk.patch_jump(jif, false_target)?;
            let false_const = chunk.add_constant(Value::Bool(false))?;
            chunk.emit(Op::Const(false_const), line);
            let end = chunk.code.len();
            chunk.patch_jump(jmp_end, end)?;
            Ok(())
        }
        // RES-083: short-circuit || desugars to `if !lhs { rhs } else { true }`.
        Node::InfixExpression { left, operator, right } if operator == "||" => {
            compile_expr(left, chunk, locals, fn_index, line)?;
            // Negate lhs so JumpIfFalse skips to "true" when lhs is truthy.
            chunk.emit(Op::Not, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // lhs was falsy → evaluate rhs
            compile_expr(right, chunk, locals, fn_index, line)?;
            let jmp_end = chunk.emit(Op::Jump(0), line);
            // true branch
            let true_target = chunk.code.len();
            chunk.patch_jump(jif, true_target)?;
            let true_const = chunk.add_constant(Value::Bool(true))?;
            chunk.emit(Op::Const(true_const), line);
            let end = chunk.code.len();
            chunk.patch_jump(jmp_end, end)?;
            Ok(())
        }
        Node::InfixExpression { left, operator, right } => {
            compile_expr(left, chunk, locals, fn_index, line)?;
            compile_expr(right, chunk, locals, fn_index, line)?;
            let op = match operator.as_str() {
                "+" => Op::Add,
                "-" => Op::Sub,
                "*" => Op::Mul,
                "/" => Op::Div,
                "%" => Op::Mod,
                // RES-083: comparison ops produce Value::Bool.
                "==" => Op::Eq,
                "!=" => Op::Neq,
                "<" => Op::Lt,
                "<=" => Op::Le,
                ">" => Op::Gt,
                ">=" => Op::Ge,
                _ => return Err(CompileError::Unsupported("non-arithmetic operator")),
            };
            chunk.emit(op, line);
            Ok(())
        }
        // RES-081: call to a top-level function. Only supports
        // calls where the callee is a bare `Identifier` — indirect
        // call through a function value (closures, lambdas) is out
        // of scope here.
        Node::CallExpression { function, arguments } => {
            let callee_name = match function.as_ref() {
                Node::Identifier { name: n, .. } => n.clone(),
                _ => return Err(CompileError::Unsupported("indirect call")),
            };
            let callee_idx = *fn_index
                .get(&callee_name)
                .ok_or_else(|| CompileError::UnknownFunction(callee_name.clone()))?;
            // Push args left-to-right so the VM can pop them in reverse
            // and assign to locals 0..arity in source order.
            for arg in arguments {
                compile_expr(arg, chunk, locals, fn_index, line)?;
            }
            chunk.emit(Op::Call(callee_idx), line);
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
        Node::Identifier { .. } => "Identifier",
        Node::IntegerLiteral { .. } => "IntegerLiteral",
        Node::FloatLiteral { .. } => "FloatLiteral",
        Node::StringLiteral { .. } => "StringLiteral",
        Node::BooleanLiteral { .. } => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::ArrayLiteral(_) => "ArrayLiteral",
        Node::IndexExpression { .. } => "IndexExpression",
        Node::IndexAssignment { .. } => "IndexAssignment",
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
        let prog = compile(&p).unwrap();
        assert_eq!(prog.main.constants.len(), 1);
        assert!(matches!(prog.main.constants[0], Value::Int(42)));
        assert_eq!(prog.main.code.first(), Some(&Op::Const(0)));
        assert!(matches!(prog.main.code.last(), Some(Op::Return)));
        assert!(prog.functions.is_empty());
    }

    #[test]
    fn compile_arith_respects_precedence() {
        let p = parse_one("2 + 3 * 4;");
        let prog = compile(&p).unwrap();
        let body: Vec<&Op> = prog
            .main
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
        let prog = compile(&p).unwrap();
        assert!(prog.main.code.iter().any(|op| matches!(op, Op::StoreLocal(0))));
    }

    #[test]
    fn compile_unknown_identifier_errors_cleanly() {
        let p = parse_one("y;");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::UnknownIdentifier(_)));
    }

    #[test]
    fn compile_unsupported_construct_is_clean_error() {
        // `for .. in` is still out of scope after RES-083. Use it as
        // the stand-in for "unsupported construct" — if we ever
        // support for-in too, pick a different canary.
        let p = parse_one("for x in [1, 2, 3] { let y = x; }");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "{:?}", err);
    }

    // ---------- RES-081 tests ----------

    #[test]
    fn compile_fn_decl_populates_functions_table() {
        let p = parse_one("fn zero() { return 0; }");
        let prog = compile(&p).unwrap();
        assert_eq!(prog.functions.len(), 1);
        assert_eq!(prog.functions[0].name, "zero");
        assert_eq!(prog.functions[0].arity, 0);
    }

    #[test]
    fn compile_call_emits_call_op() {
        let p = parse_one("fn zero() { return 0; } zero();");
        let prog = compile(&p).unwrap();
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(0))),
            "expected Call(0) in main.code: {:?}",
            prog.main.code
        );
    }

    #[test]
    fn compile_unknown_function_call_errors() {
        let p = parse_one("nope();");
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::UnknownFunction(_)), "{:?}", err);
    }

    #[test]
    fn compile_fn_with_params_maps_them_to_first_locals() {
        let p = parse_one("fn sq(int n) { return n * n; }");
        let prog = compile(&p).unwrap();
        let f = &prog.functions[0];
        assert_eq!(f.arity, 1);
        // Inside the body, `n` is local 0. The emitted code should
        // LoadLocal(0) twice before Mul.
        let load_count = f.chunk.code.iter().filter(|op| matches!(op, Op::LoadLocal(0))).count();
        assert_eq!(load_count, 2, "expected two LoadLocal(0) for n*n: {:?}", f.chunk.code);
    }

    #[test]
    fn compile_too_many_params_errors() {
        // 256 params — over the u8 limit.
        let params: Vec<String> = (0..256).map(|i| format!("int p{}", i)).collect();
        let src = format!("fn big({}) {{ return 0; }}", params.join(", "));
        let p = parse_one(&src);
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "{:?}", err);
    }
}
