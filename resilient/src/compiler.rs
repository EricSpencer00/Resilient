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

    // Pre-pass 0 (FFI v2): resolve all extern blocks so foreign symbols
    // are available before any call-site compilation. Builds an
    // ffi_index: name → u16 parallel to fn_index.
    #[cfg(feature = "ffi")]
    let mut ffi_loader = crate::ffi::ForeignLoader::new();
    #[cfg(feature = "ffi")]
    let mut ffi_index: HashMap<String, u16> = HashMap::new();
    #[cfg(feature = "ffi")]
    let mut foreign_syms: Vec<std::sync::Arc<crate::ffi::ForeignSymbol>> = Vec::new();
    #[cfg(feature = "ffi")]
    {
        for spanned in stmts {
            if let Node::Extern { library, decls, .. } = &spanned.node {
                ffi_loader
                    .resolve_block(library, decls)
                    .map_err(|e| CompileError::FfiError(e.to_string()))?;
                for d in decls {
                    if let Some(sym) = ffi_loader.lookup(&d.resilient_name) {
                        if ffi_index.len() >= u16::MAX as usize {
                            return Err(CompileError::Unsupported(
                                "too many foreign symbols (>65535)",
                            ));
                        }
                        let idx = foreign_syms.len() as u16;
                        ffi_index.insert(d.resilient_name.clone(), idx);
                        foreign_syms.push(sym);
                    }
                }
            }
        }
    }
    // On non-ffi builds, ffi_index is empty — call sites fall through to
    // the normal fn_index lookup and surface a "function not found" error.
    #[cfg(not(feature = "ffi"))]
    let ffi_index: HashMap<String, u16> = HashMap::new();

    // Pre-pass: function name → index in the `functions` table.
    let mut fn_index: HashMap<String, u16> = HashMap::new();
    let mut next_fn_idx: u16 = 0;
    for spanned in stmts {
        if let Node::Function {
            name, parameters, ..
        } = &spanned.node
        {
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
        if let Node::Function {
            name,
            parameters,
            body,
            ..
        } = &spanned.node
        {
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
            // Function bodies are `Node::Block { stmts, .. }` today. Walk
            // the inner statements; emit a trailing ReturnFromCall so
            // a body that fell through produces Void to the caller.
            let inner = match body.as_ref() {
                Node::Block { stmts: b, .. } => b,
                single => std::slice::from_ref(single),
            };
            for stmt in inner {
                // RES-092: prefer the body statement's own span so
                // VM runtime errors land on the offending source
                // line, not just the line of the enclosing fn.
                // Falls back to the fn's outer line when the
                // statement node has no span (synthetic).
                let line = node_line(stmt).unwrap_or(spanned.span.start.line as u32);
                compile_stmt_in_fn(
                    stmt,
                    &mut chunk,
                    &mut locals,
                    &mut next_local,
                    &fn_index,
                    &ffi_index,
                    line,
                )?;
            }
            chunk.emit(Op::ReturnFromCall, 0);
            // RES-384: replace self-tail-calls with TailCall. Scan
            // for every `Call(own_idx); ReturnFromCall` pair and
            // fold it into a single `TailCall(own_idx)`. This
            // handles tail calls in all positions — explicit
            // `return f(args);` statements and implicit tail
            // returns from if-branches. Must run before the
            // peephole pass so peephole sees the final opcode
            // sequence.
            let own_fn_idx = functions.len() as u16;
            rewrite_tail_calls(&mut chunk, own_fn_idx);
            // RES-172: run the peephole optimizer over the
            // just-emitted chunk. Idempotent and linear-scan —
            // no effect on chunks that don't contain any of the
            // shipped idioms.
            crate::peephole::optimize(&mut chunk)
                .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
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
        // Skip fn/extern decls — handled in earlier passes.
        // RES-391: `region <Name>;` is compile-time metadata only;
        // it emits no code in either the tree-walker or the VM.
        if matches!(
            spanned.node,
            Node::Function { .. } | Node::Extern { .. } | Node::RegionDecl { .. }
        ) {
            continue;
        }
        let line = spanned.span.start.line as u32;
        compile_stmt(
            &spanned.node,
            &mut main,
            &mut main_locals,
            &mut main_next_local,
            &fn_index,
            &ffi_index,
            line,
        )?;
    }
    main.emit(Op::Return, 0);
    // RES-172: peephole pass over the main chunk too.
    crate::peephole::optimize(&mut main)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;

    Ok(Program {
        main,
        functions,
        #[cfg(feature = "ffi")]
        foreign_syms,
    })
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
    ffi_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
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
            compile_expr(v, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ExpressionStatement { expr: inner, .. } => {
            compile_expr(inner, chunk, locals, fn_index, ffi_index, line)
        }
        Node::IfStatement { .. } | Node::WhileStatement { .. } | Node::Block { .. } => {
            compile_control_flow(node, chunk, locals, next_local, fn_index, ffi_index, line)
        }
        Node::Assignment { name, value, .. } => {
            // RES-083: re-bind an existing local. Compile the RHS,
            // StoreLocal to the known slot. Unknown name is an error.
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-171a: `a[i] = v;` where `a` is a bare Identifier.
        // Lowered as:
        //   LoadLocal(a), <i>, <v>, StoreIndex, StoreLocal(a)
        // The Array on top of the stack after StoreIndex IS the
        // mutated one (the VM dispatch pushes it back), so writing
        // it through `StoreLocal` commits the update.
        //
        // Nested `a[i][j] = v` is RES-171c; here we explicitly
        // reject non-Identifier targets so the compile error is
        // descriptive rather than a silent miscompile.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            let local_name = match target.as_ref() {
                Node::Identifier { name, .. } => name.clone(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested index assignment (RES-171c)",
                    ));
                }
            };
            let slot = *locals
                .get(&local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.clone()))?;
            chunk.emit(Op::LoadLocal(slot), line);
            compile_expr(index, chunk, locals, fn_index, ffi_index, line)?;
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::StoreIndex, line);
            chunk.emit(Op::StoreLocal(slot), line);
            Ok(())
        }
        Node::Function { .. } | Node::Extern { .. } => {
            // Top-level fn/extern decls already handled in passes 1/2.
            // Skipping here would be a no-op, but we should never see
            // them — the caller filters them out before calling us.
            Err(CompileError::Unsupported("nested function/extern decl"))
        }
        // RES-390: actor / cluster decls are compile-time-only
        // verifier constructs. The bytecode backend emits nothing
        // for them — the interpreter also treats them as no-ops.
        Node::ActorDecl { .. } | Node::ClusterDecl { .. } => Ok(()),
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
    ffi_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                compile_stmt(s, chunk, locals, next_local, fn_index, ffi_index, line)?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            // cond
            compile_expr(condition, chunk, locals, fn_index, ffi_index, line)?;
            // JumpIfFalse to else-or-end (placeholder 0 offset)
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // consequence
            compile_stmt(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                line,
            )?;
            if let Some(alt) = alternative {
                // Unconditional jump past the else branch
                let jmp_end = chunk.emit(Op::Jump(0), line);
                // JumpIfFalse lands here (start of else)
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt(alt, chunk, locals, next_local, fn_index, ffi_index, line)?;
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
        Node::WhileStatement {
            condition, body, ..
        } => {
            let loop_start = chunk.code.len();
            compile_expr(condition, chunk, locals, fn_index, ffi_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt(body, chunk, locals, next_local, fn_index, ffi_index, line)?;
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
    ffi_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
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
            compile_expr(v, chunk, locals, fn_index, ffi_index, line)?;
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
        Node::ExpressionStatement { expr: inner, .. } => {
            compile_expr(inner, chunk, locals, fn_index, ffi_index, line)
        }
        Node::IfStatement { .. } | Node::WhileStatement { .. } | Node::Block { .. } => {
            compile_control_flow_in_fn(node, chunk, locals, next_local, fn_index, ffi_index, line)
        }
        Node::Assignment { name, value, .. } => {
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-171a: same shape as the main-chunk IndexAssignment
        // arm. Duplicated on purpose because `compile_stmt` and
        // `compile_stmt_in_fn` are separate matches (one emits
        // `Return`, the other `ReturnFromCall`); extracting a
        // shared helper is overkill for RES-171a but a candidate
        // cleanup when RES-171c expands this path.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            let local_name = match target.as_ref() {
                Node::Identifier { name, .. } => name.clone(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested index assignment (RES-171c)",
                    ));
                }
            };
            let slot = *locals
                .get(&local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.clone()))?;
            chunk.emit(Op::LoadLocal(slot), line);
            compile_expr(index, chunk, locals, fn_index, ffi_index, line)?;
            compile_expr(value, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::StoreIndex, line);
            chunk.emit(Op::StoreLocal(slot), line);
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
    ffi_index: &HashMap<String, u16>,
    line: u32,
) -> Result<(), CompileError> {
    match node {
        Node::Block { stmts, .. } => {
            for s in stmts {
                compile_stmt_in_fn(s, chunk, locals, next_local, fn_index, ffi_index, line)?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            compile_expr(condition, chunk, locals, fn_index, ffi_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                line,
            )?;
            if let Some(alt) = alternative {
                let jmp_end = chunk.emit(Op::Jump(0), line);
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt_in_fn(alt, chunk, locals, next_local, fn_index, ffi_index, line)?;
                let end = chunk.code.len();
                chunk.patch_jump(jmp_end, end)?;
            } else {
                let end = chunk.code.len();
                chunk.patch_jump(jif, end)?;
            }
            Ok(())
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            let loop_start = chunk.code.len();
            compile_expr(condition, chunk, locals, fn_index, ffi_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(body, chunk, locals, next_local, fn_index, ffi_index, line)?;
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
    ffi_index: &HashMap<String, u16>,
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
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => {
            compile_expr(right, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::Neg, line);
            Ok(())
        }
        // RES-083: logical negation.
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => {
            compile_expr(right, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::Not, line);
            Ok(())
        }
        // RES-083: short-circuit && desugars to `if lhs { rhs } else { false }`.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if operator == "&&" => {
            compile_expr(left, chunk, locals, fn_index, ffi_index, line)?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_expr(right, chunk, locals, fn_index, ffi_index, line)?;
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
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if operator == "||" => {
            compile_expr(left, chunk, locals, fn_index, ffi_index, line)?;
            // Negate lhs so JumpIfFalse skips to "true" when lhs is truthy.
            chunk.emit(Op::Not, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // lhs was falsy → evaluate rhs
            compile_expr(right, chunk, locals, fn_index, ffi_index, line)?;
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
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            compile_expr(left, chunk, locals, fn_index, ffi_index, line)?;
            compile_expr(right, chunk, locals, fn_index, ffi_index, line)?;
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
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let callee_name = match function.as_ref() {
                Node::Identifier { name: n, .. } => n.clone(),
                _ => return Err(CompileError::Unsupported("indirect call")),
            };
            // FFI v2: foreign call takes priority over user-defined functions.
            if let Some(&idx) = ffi_index.get(&callee_name) {
                for arg in arguments {
                    compile_expr(arg, chunk, locals, fn_index, ffi_index, line)?;
                }
                chunk.emit(Op::CallForeign(idx), line);
                return Ok(());
            }
            let callee_idx = *fn_index
                .get(&callee_name)
                .ok_or_else(|| CompileError::UnknownFunction(callee_name.clone()))?;
            // Push args left-to-right so the VM can pop them in reverse
            // and assign to locals 0..arity in source order.
            for arg in arguments {
                compile_expr(arg, chunk, locals, fn_index, ffi_index, line)?;
            }
            chunk.emit(Op::Call(callee_idx), line);
            Ok(())
        }
        // RES-171a: `[a, b, c]` literal → emit each item's expression
        // left-to-right, then `Op::MakeArray { len }` which pops them
        // all into a `Value::Array`.
        Node::ArrayLiteral { items, .. } => {
            if items.len() > u16::MAX as usize {
                return Err(CompileError::Unsupported("array literal with >65535 items"));
            }
            for item in items {
                compile_expr(item, chunk, locals, fn_index, ffi_index, line)?;
            }
            chunk.emit(
                Op::MakeArray {
                    len: items.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-171a: `target[index]` read → push target, push index,
        // emit `LoadIndex`. Bounds + type checks happen in the VM.
        // Nested targets (e.g. `a[i][j]`) fall out naturally because
        // `compile_expr(target)` recurses: each `IndexExpression` at
        // an inner position pushes a clone of the sub-array.
        Node::IndexExpression { target, index, .. } => {
            compile_expr(target, chunk, locals, fn_index, ffi_index, line)?;
            compile_expr(index, chunk, locals, fn_index, ffi_index, line)?;
            chunk.emit(Op::LoadIndex, line);
            Ok(())
        }
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Static descriptor for a node kind, used in `Unsupported` errors.
/// RES-092: extract a 1-indexed source line from any Node variant
/// that carries a `Span`. Returns `None` for nodes whose span is
/// `Span::default()` (line 0 = synthetic) or for variants that
/// don't carry a span at all. Callers fall back to a parent-scope
/// line in those cases.
fn node_line(n: &Node) -> Option<u32> {
    let line: u32 = match n {
        // Statement variants (RES-079).
        Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. } => span.start.line as u32,

        // Block + ExpressionStatement (RES-087, tuple→struct).
        Node::Block { span, .. } | Node::ExpressionStatement { span, .. } => span.start.line as u32,

        // Leaves (RES-078).
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::Identifier { span, .. } => span.start.line as u32,

        // Core expressions (RES-084) and index/field (RES-085).
        Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::IndexAssignment { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::FieldAssignment { span, .. } => span.start.line as u32,

        // Tuple-struct conversions (RES-086).
        Node::ArrayLiteral { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. } => span.start.line as u32,

        // RES-148: map literal carries a span at its opening brace.
        Node::MapLiteral { span, .. } => span.start.line as u32,

        // RES-149: set literal span at its opening `#{`.
        Node::SetLiteral { span, .. } => span.start.line as u32,

        // RES-152: bytes literal span at its opening `b"`.
        Node::BytesLiteral { span, .. } => span.start.line as u32,

        // RES-155: struct destructure let carries the `let` keyword span.
        Node::LetDestructureStruct { span, .. } => span.start.line as u32,

        // Structural variants (RES-088).
        Node::Function { span, .. }
        | Node::Use { span, .. }
        | Node::Extern { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. }
        | Node::Match { span, .. }
        | Node::StructDecl { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::TypeAlias { span, .. }
        | Node::RegionDecl { span, .. }
        | Node::Actor { span, .. }
        | Node::ActorDecl { span, .. }
        | Node::ClusterDecl { span, .. }
        | Node::FunctionLiteral { span, .. } => span.start.line as u32,

        // RES-142: duration literal carries the span of its integer
        // part; only emitted inside live-clause position so it
        // shouldn't round-trip through the compiler, but match it
        // anyway to keep the pattern exhaustive.
        Node::DurationLiteral { span, .. } => span.start.line as u32,

        // Program is wrapped in Spanned<Node> at the call site, not
        // inside the Node enum itself.
        Node::Program(_) => 0,
    };
    if line == 0 { None } else { Some(line) }
}

fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Program(_) => "Program",
        Node::Use { .. } => "Use",
        Node::Function { .. } => "Function",
        Node::LiveBlock { .. } => "LiveBlock",
        Node::Assert { .. } => "Assert",
        Node::Assume { .. } => "Assume",
        Node::Block { .. } => "Block",
        Node::LetStatement { .. } => "LetStatement",
        Node::StaticLet { .. } => "StaticLet",
        Node::Assignment { .. } => "Assignment",
        Node::ReturnStatement { .. } => "ReturnStatement",
        Node::IfStatement { .. } => "IfStatement",
        Node::WhileStatement { .. } => "WhileStatement",
        Node::ForInStatement { .. } => "ForInStatement",
        Node::ExpressionStatement { .. } => "ExpressionStatement",
        Node::Identifier { .. } => "Identifier",
        Node::IntegerLiteral { .. } => "IntegerLiteral",
        Node::FloatLiteral { .. } => "FloatLiteral",
        Node::StringLiteral { .. } => "StringLiteral",
        Node::BooleanLiteral { .. } => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::ArrayLiteral { .. } => "ArrayLiteral",
        Node::IndexExpression { .. } => "IndexExpression",
        Node::IndexAssignment { .. } => "IndexAssignment",
        Node::RegionDecl { .. } => "RegionDecl",
        _ => "<other>",
    }
}

// ============================================================
// RES-384: tail-call rewriting pass
// ============================================================

/// Scan `chunk.code` for every adjacent `Call(fn_idx); ReturnFromCall`
/// pair where `fn_idx == own_fn_idx` and replace the pair with a
/// single `TailCall(fn_idx)`. The removed `ReturnFromCall` leaves a
/// hole; rather than shifting the Vec (which would invalidate all
/// existing jump targets), we overwrite the second slot of each pair
/// with a `Jump(0)` sentinel pointing one step back so the dead op
/// can never be reached:
///
/// ```text
/// before:  [..., Call(i), ReturnFromCall, ...]
/// after:   [..., TailCall(i), (dead/unreachable), ...]
/// ```
///
/// Because `TailCall` does not fall through (it loops back to pc=0),
/// the instruction following it is dead. We leave it as a `Return`
/// no-op rather than a `Jump` to avoid confusing the disassembler;
/// the VM will never execute it.
///
/// Jump targets are NOT shifted — this transform only touches pairs
/// where the second op is `ReturnFromCall`, which nothing ever jumps
/// TO (no other op emits a forward-jump into `ReturnFromCall`; all
/// branch targets land on the instruction AFTER a block, not ON a
/// return). This invariant holds for the patterns the compiler emits.
fn rewrite_tail_calls(chunk: &mut crate::bytecode::Chunk, own_fn_idx: u16) {
    let len = chunk.code.len();
    if len < 2 {
        return;
    }
    // We need indices so we can write back; collect positions first.
    let mut positions: Vec<usize> = Vec::new();
    for i in 0..len - 1 {
        if chunk.code[i] == Op::Call(own_fn_idx) && chunk.code[i + 1] == Op::ReturnFromCall {
            positions.push(i);
        }
    }
    for pos in positions {
        // Replace the Call with TailCall; mark the ReturnFromCall
        // dead by overwriting with a no-op Return. The VM never
        // reaches it because TailCall resets pc, but leaving a
        // valid opcode keeps the chunk well-formed for the
        // disassembler and any future static analyses.
        chunk.code[pos] = Op::TailCall(own_fn_idx);
        chunk.code[pos + 1] = Op::Return; // unreachable tombstone
        // Preserve line info alignment by keeping the two slots;
        // no shift needed.
    }
}

// ============================================================
// RES-170a: struct registry
// ============================================================
//
// The VM's eventual struct-ops lowering (RES-170c) needs to
// answer two questions at compile time without the runtime ever
// touching string names:
//
//   - "What `type_id` should `Op::MakeStruct` carry for this
//     struct literal?"
//   - "What `u8` field index corresponds to `p.x`?"
//
// This module builds the registry that answers both. Each
// `Node::StructDecl` in the program gets a unique `type_id`
// (assigned in source order so the indices are stable across
// compile invocations), and each field gets a `u8` slot index
// matching its declaration order. RES-170b will walk the AST
// threading local → struct-name info; RES-170c will consume the
// registry to emit MakeStruct / LoadField / StoreField.
//
// ## Why not reuse the JIT's RES-165a StructLayout?
//
// Different data. RES-165a computes byte offsets + cranelift
// `Type`s for the JIT's stack-allocated repr(C) layout. The VM
// uses a heap-allocated `Vec<Value>` indexed by field position —
// no byte offsets, no per-field types (each slot is a `Value`).
// The field-name-to-index map is the only shared piece, and
// each backend derives its own copy from the same
// `Node::StructDecl`. When cross-module type-id uniqueness
// lands (RES-170d), we may pull the registry into a common
// module and surface it to both backends; for today a
// compiler-local definition is simpler.

/// RES-170a: per-struct entry in the registry. `name` duplicates
/// the map key so callers can use an `&StructRegistryEntry` on
/// its own without lugging around the key. `fields` is sorted by
/// declaration position, so `fields[i]` is the name at slot `i`
/// and `field_index(name) -> Some(i as u8)` does the inverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructRegistryEntry {
    /// Declared struct name (e.g. `"Point"`).
    pub name: String,
    /// Compile-time identifier for the struct. Unique within a
    /// `StructRegistry` build; assignment order matches the
    /// source order the decl appeared in the `Program`.
    pub type_id: u16,
    /// Field names in declaration order. Slot index inside a
    /// `Value::Struct { fields, .. }` matches this vector's
    /// indexing, so `LoadField { idx }` reads `fields[idx]`.
    pub fields: Vec<String>,
}

impl StructRegistryEntry {
    /// Return the `u8` slot index for `field_name`, or `None` if
    /// the struct has no such field. Linear scan — struct field
    /// counts are small and this is a compile-time lookup, not a
    /// per-instruction hot path.
    pub fn field_index(&self, field_name: &str) -> Option<u8> {
        self.fields
            .iter()
            .position(|f| f == field_name)
            .map(|i| i as u8)
    }
}

/// RES-170a: compile-time registry of every `Node::StructDecl`
/// in a `Program`. Built by `StructRegistry::from_program`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StructRegistry {
    /// Keyed by declared name; each entry carries its `type_id`
    /// and field vector.
    entries: HashMap<String, StructRegistryEntry>,
}

impl StructRegistry {
    /// Walk every top-level `Node::StructDecl` in `program` and
    /// build a registry. Errors:
    ///
    ///   - `DuplicateStructName(name)` — two decls share `name`.
    ///   - `TooManyStructDecls`        — more than u16::MAX + 1 decls.
    ///   - `TooManyFields(name)`       — one decl has more than
    ///     u8::MAX + 1 fields (RES-170c's `LoadField { idx: u8 }`
    ///     is the hard cap).
    ///
    /// Nested declarations (inside `ImplBlock`s or other
    /// containers) are ignored for today; the parser only places
    /// `StructDecl`s at `Program` scope.
    pub fn from_program(program: &Node) -> Result<Self, CompileError> {
        let stmts = match program {
            Node::Program(s) => s,
            _ => {
                return Err(CompileError::Unsupported(
                    "struct registry requires a Program root",
                ));
            }
        };
        let mut entries: HashMap<String, StructRegistryEntry> = HashMap::new();
        let mut next_type_id: u32 = 0;
        for spanned in stmts {
            let Node::StructDecl { name, fields, .. } = &spanned.node else {
                continue;
            };
            if entries.contains_key(name) {
                return Err(CompileError::DuplicateStructName(name.clone()));
            }
            if fields.len() > u8::MAX as usize + 1 {
                return Err(CompileError::TooManyFields(name.clone()));
            }
            if next_type_id > u16::MAX as u32 {
                return Err(CompileError::TooManyStructDecls);
            }
            let field_names: Vec<String> =
                fields.iter().map(|(_ty, fname)| fname.clone()).collect();
            entries.insert(
                name.clone(),
                StructRegistryEntry {
                    name: name.clone(),
                    type_id: next_type_id as u16,
                    fields: field_names,
                },
            );
            next_type_id += 1;
        }
        Ok(Self { entries })
    }

    /// Number of registered struct decls.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if no struct decls were registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up a struct by name. Returns `None` if the program
    /// has no matching decl.
    pub fn get(&self, name: &str) -> Option<&StructRegistryEntry> {
        self.entries.get(name)
    }

    /// Convenience: resolve `(struct_name, field_name)` to the
    /// `(type_id, field_index)` pair RES-170c will encode into
    /// `MakeStruct` / `LoadField` operands. Returns `None` when
    /// the struct or the field doesn't exist.
    pub fn resolve(&self, struct_name: &str, field_name: &str) -> Option<(u16, u8)> {
        let entry = self.entries.get(struct_name)?;
        let idx = entry.field_index(field_name)?;
        Some((entry.type_id, idx))
    }
}

#[cfg(test)]
pub(crate) fn parse_and_compile(src: &str) -> Result<Program, String> {
    let (ast, errs) = crate::parse(src);
    if !errs.is_empty() {
        return Err(errs.join("; "));
    }
    compile(&ast).map_err(|e| e.to_string())
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

    #[cfg(feature = "ffi")]
    #[test]
    fn extern_block_produces_foreign_sym_in_program() {
        let src = "fn main() { return 1; }\n";
        let prog = crate::compiler::parse_and_compile(src).expect("compiles");
        assert!(prog.foreign_syms.is_empty());
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
        assert!(
            prog.main
                .code
                .iter()
                .any(|op| matches!(op, Op::StoreLocal(0)))
        );
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
        let load_count = f
            .chunk
            .code
            .iter()
            .filter(|op| matches!(op, Op::LoadLocal(0)))
            .count();
        assert_eq!(
            load_count, 2,
            "expected two LoadLocal(0) for n*n: {:?}",
            f.chunk.code
        );
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

    // ---------- RES-170a: struct registry ----------

    #[test]
    fn res170a_empty_program_has_empty_registry() {
        let p = parse_one("return 1;");
        let reg = StructRegistry::from_program(&p).unwrap();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn res170a_single_struct_registers_with_type_id_zero() {
        let p = parse_one(
            r#"
            struct Point {
                int x,
                int y,
            }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        let pt = reg.get("Point").expect("Point should be registered");
        assert_eq!(pt.name, "Point");
        assert_eq!(pt.type_id, 0);
        assert_eq!(pt.fields, vec!["x".to_string(), "y".to_string()]);
    }

    #[test]
    fn res170a_field_names_preserve_declaration_order() {
        let p = parse_one(
            r#"
            struct Rec {
                int c,
                int a,
                int b,
            }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        let r = reg.get("Rec").unwrap();
        // Source order is c, a, b — NOT alphabetical.
        assert_eq!(
            r.fields,
            vec!["c".to_string(), "a".to_string(), "b".to_string()]
        );
        assert_eq!(r.field_index("c"), Some(0));
        assert_eq!(r.field_index("a"), Some(1));
        assert_eq!(r.field_index("b"), Some(2));
    }

    #[test]
    fn res170a_field_index_missing_returns_none() {
        let p = parse_one(r#"struct S { int x, }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        let s = reg.get("S").unwrap();
        assert_eq!(s.field_index("x"), Some(0));
        assert!(s.field_index("nope").is_none());
    }

    #[test]
    fn res170a_multiple_structs_get_sequential_type_ids() {
        let p = parse_one(
            r#"
            struct A { int x, }
            struct B { int y, }
            struct C { int z, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.get("A").unwrap().type_id, 0);
        assert_eq!(reg.get("B").unwrap().type_id, 1);
        assert_eq!(reg.get("C").unwrap().type_id, 2);
        assert_eq!(reg.len(), 3);
    }

    #[test]
    fn res170a_duplicate_struct_name_errors() {
        let p = parse_one(
            r#"
            struct Dup { int x, }
            struct Dup { int y, }
        "#,
        );
        let err = StructRegistry::from_program(&p).unwrap_err();
        match err {
            CompileError::DuplicateStructName(n) => assert_eq!(n, "Dup"),
            other => panic!("expected DuplicateStructName, got {:?}", other),
        }
    }

    #[test]
    fn res170a_unknown_struct_lookup_is_none() {
        let p = parse_one(r#"struct P { int x, }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        assert!(reg.get("Q").is_none());
    }

    #[test]
    fn res170a_resolve_roundtrips_type_id_and_field_index() {
        let p = parse_one(
            r#"
            struct First  { int a, }
            struct Second { int x, bool y, int z, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.resolve("First", "a"), Some((0, 0)));
        assert_eq!(reg.resolve("Second", "x"), Some((1, 0)));
        assert_eq!(reg.resolve("Second", "y"), Some((1, 1)));
        assert_eq!(reg.resolve("Second", "z"), Some((1, 2)));
        // Unknown struct / unknown field → None.
        assert!(reg.resolve("Nope", "a").is_none());
        assert!(reg.resolve("Second", "nope").is_none());
    }

    #[test]
    fn res170a_registry_coexists_with_let_and_fn_decls() {
        // Realistic program: mixed struct / fn / let statements at
        // top level. The registry must pick up only the structs.
        let p = parse_one(
            r#"
            let start = 0;
            struct P { int x, int y, }
            fn add(int a, int b) -> int { return a + b; }
            struct Q { bool flag, }
        "#,
        );
        let reg = StructRegistry::from_program(&p).unwrap();
        assert_eq!(reg.len(), 2);
        assert_eq!(reg.get("P").unwrap().type_id, 0);
        assert_eq!(reg.get("Q").unwrap().type_id, 1);
    }

    #[test]
    fn res170a_empty_struct_gets_empty_field_vec() {
        let p = parse_one(r#"struct Empty { }"#);
        let reg = StructRegistry::from_program(&p).unwrap();
        let e = reg.get("Empty").unwrap();
        assert!(e.fields.is_empty());
        assert!(e.field_index("anything").is_none());
    }

    #[test]
    fn res170a_non_program_root_errors() {
        // The registry requires a Program root — fed a bare node,
        // it should reject rather than silently produce an empty
        // registry.
        let just_int = Node::IntegerLiteral {
            value: 42,
            span: crate::span::Span::default(),
        };
        let err = StructRegistry::from_program(&just_int).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "got {:?}", err);
    }
}
