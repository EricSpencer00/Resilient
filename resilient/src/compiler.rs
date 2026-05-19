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

/// Tracks break/continue patch sites accumulated while compiling a loop body.
///
/// Created fresh for each While/ForIn loop. Nested loops create their own
/// inner LoopState, shadowing the outer one, so `break`/`continue` always
/// target the *innermost* enclosing loop — matching the tree-walker's
/// `Value::Break`/`Value::Continue` bubble-up semantics.
struct LoopState {
    /// PC of the back-edge target. For `while` this is the condition check;
    /// for `for-in` it's the index-increment code (set after body compilation
    /// via `set_continue_target`).
    continue_target: usize,
    /// `Jump(0)` instruction indices emitted for `break` that need to be
    /// patched to the loop-exit PC once the loop is fully compiled.
    break_patches: Vec<usize>,
    /// `Jump(0)` instruction indices emitted for `continue` that need to be
    /// patched to `continue_target`. Used by `for-in` loops, where the target
    /// is not yet known when the body is compiled.
    continue_patches: Vec<usize>,
}

impl LoopState {
    fn new(continue_target: usize) -> Self {
        LoopState {
            continue_target,
            break_patches: Vec::new(),
            continue_patches: Vec::new(),
        }
    }

    /// Retroactively fix up all `continue` patch sites — called by `for-in`
    /// after the index-increment code is in place.
    fn set_continue_target(&mut self, target: usize) {
        self.continue_target = target;
    }
}

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
    //
    // RES-1577: pre-size `ffi_index` and `foreign_syms` to the total
    // extern-decl count. Same shape as RES-1461's `fn_index` pre-size;
    // skips the default-bucket rehash chain for programs with many
    // FFI symbols. One linear pass over `stmts` to count, mirroring
    // the existing `fn_count` block below.
    #[cfg(feature = "ffi")]
    let ffi_count: usize = stmts
        .iter()
        .filter_map(|s| match &s.node {
            Node::Extern { decls, .. } => Some(decls.len()),
            _ => None,
        })
        .sum();
    #[cfg(feature = "ffi")]
    let mut ffi_loader = crate::ffi::ForeignLoader::new();
    #[cfg(feature = "ffi")]
    let mut ffi_index: HashMap<String, u16> = HashMap::with_capacity(ffi_count);
    #[cfg(feature = "ffi")]
    let mut foreign_syms: Vec<std::sync::Arc<crate::ffi::ForeignSymbol>> =
        Vec::with_capacity(ffi_count);
    #[cfg(feature = "ffi")]
    {
        for spanned in stmts {
            if let Node::Extern { library, decls, .. } = &spanned.node {
                ffi_loader
                    .resolve_block(library, decls)
                    .map_err(|e| CompileError::FfiError(e.to_string()))?;
                for d in decls {
                    if d.is_variadic {
                        return Err(CompileError::Unsupported(
                            "variadic extern calls are supported by the tree-walker only",
                        ));
                    }
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

    // RES-1461: pre-size `fn_index` and `functions` to the actual
    // top-level Function count. The previous shape used
    // `HashMap::new()` / `Vec::new()` and grew them entry-by-entry,
    // triggering reallocations as they crossed the default-bucket
    // boundaries. Most programs have at least a handful of functions;
    // a one-shot count is essentially free (linear over top-level
    // statements, same shape as the loop below). Mirrors RES-1365's
    // struct-fields pre-size pattern and RES-1399's actor
    // resolved_fields pre-size.
    let fn_count = stmts
        .iter()
        .filter(|s| matches!(&s.node, Node::Function { .. }))
        .count();

    // Pre-pass: function name → index in the `functions` table.
    let mut fn_index: HashMap<String, u16> = HashMap::with_capacity(fn_count);
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
    let mut functions: Vec<Function> = Vec::with_capacity(fn_count);
    for spanned in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            ..
        } = &spanned.node
        {
            let arity = parameters.len() as u8;
            // RES-1720: pre-size the Chunk's opcode buffers. 128 fits
            // the median Resilient fn body (~50-200 opcodes) without
            // needing a Vec realloc; tiny functions overshoot by < 1
            // KB. Same shape as RES-1716 / RES-1714 / RES-1718.
            let mut chunk = Chunk::with_capacity(128);
            // Parameters occupy locals 0..arity. Map each param name
            // to its slot; additional `let` bindings in the body bump
            // `next_local` from there.
            //
            // RES-1575: pre-size to `parameters.len() * 2` (params + a
            // heuristic body-let allowance), floored at 8 so empty
            // fns don't double-rehash from the default 0→4→8 grow
            // path. Compiling N functions previously paid up to two
            // rehashes per fn; one upfront `with_capacity` avoids
            // them on the hot per-fn loop.
            let mut locals: HashMap<String, u16> =
                HashMap::with_capacity(parameters.len().saturating_mul(2).max(8));
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
                    &mut functions,
                    &mut next_fn_idx,
                    line,
                    None,
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
            // RES-298: constant-fold pure expressions over literals
            // BEFORE the peephole pass. Folding turns sequences like
            // `Const Const Add` into a single `Const`, which the
            // peephole's identity-fold rules (`Const 0; Add`,
            // `Const 1; Mul`, …) can then act on.
            crate::const_fold::optimize_if_enabled(&mut chunk)
                .map_err(|_| CompileError::InternalError("constant folder failed"))?;
            // RES-172: run the peephole optimizer over the
            // just-emitted chunk. Idempotent and linear-scan —
            // no effect on chunks that don't contain any of the
            // shipped idioms.
            crate::peephole::optimize(&mut chunk)
                .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
            // RES-297: dead code elimination — remove unreachable ops
            // after returns and fold constant-condition branches.
            crate::dce::eliminate(&mut chunk);
            functions.push(Function {
                name: name.clone(),
                arity,
                chunk,
                local_count: next_local,
            });
        }
    }

    // Pass 3: compile the remaining top-level statements into `main`.
    // RES-1720: pre-size — top-level body usually emits a handful of
    // setup ops + per-stmt calls. 64 fits the common case.
    let mut main = Chunk::with_capacity(64);
    // RES-1716: pre-size `main_locals` — same shape as RES-1461 for
    // `fn_index`. Top-level `let` / `const` / `static` bindings flow
    // into this map; typical programs have 5-20 entries. Pre-sizing
    // to 16 fits the common case in one allocation.
    let mut main_locals: HashMap<String, u16> = HashMap::with_capacity(16);
    let mut main_next_local: u16 = 0;
    for spanned in stmts {
        // Skip fn/extern decls — handled in earlier passes.
        // RES-391: `region <Name>;` is compile-time metadata only;
        // it emits no code in either the tree-walker or the VM.
        // RES-335: `struct <Name> { ... }` decls are likewise
        // compile-time metadata — the `StructLiteral` opcode carries
        // the type name directly and does not consult a decl table.
        if matches!(
            spanned.node,
            Node::Function { .. }
                | Node::Extern { .. }
                | Node::RegionDecl { .. }
                | Node::StructDecl { .. }
                // RES-319: newtype declarations are compile-time metadata;
                // constructor calls are already lowered to NewtypeConstruct.
                | Node::NewtypeDecl { .. }
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
            &mut functions,
            &mut next_fn_idx,
            line,
            None,
        )?;
    }
    main.emit(Op::Return, 0);
    // RES-298: constant fold the main chunk before peephole runs.
    crate::const_fold::optimize_if_enabled(&mut main)
        .map_err(|_| CompileError::InternalError("constant folder failed"))?;
    // RES-172: peephole pass over the main chunk too.
    crate::peephole::optimize(&mut main)
        .map_err(|_| CompileError::InternalError("peephole optimizer failed"))?;
    // RES-297: dead code elimination over main chunk.
    crate::dce::eliminate(&mut main);

    let mut prog = Program {
        main,
        functions,
        #[cfg(feature = "ffi")]
        foreign_syms,
    };
    // RES-365: function inlining pass over the assembled program.
    // Replaces `Op::Call(idx)` to small leaf functions with the
    // callee's bytecode body, eliminating call-frame overhead. Gated
    // behind `RESILIENT_INLINE=1` so default behavior stays
    // bit-identical (matches the `const_fold::optimize_if_enabled`
    // discipline — the existing test suite pins specific opcode
    // sequences that inlining would change).
    crate::inline::optimize_if_enabled(&mut prog)
        .map_err(|_| CompileError::InternalError("inliner failed"))?;
    Ok(prog)
}

/// Compile a top-level (main-chunk) statement. Bare expression
/// statements leak their value onto the operand stack, which `Return`
/// picks up as the program result — useful for the RES-076 smoke
/// test that parses `2 + 3 * 4;`.
///
/// `loop_state` is `Some` when this statement is nested inside a loop
/// body and carries the break/continue patch sites for that loop.
#[allow(clippy::too_many_arguments)]
fn compile_stmt(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_state: Option<&mut LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-401: `let (a, b, c) = expr;` in top-level (main chunk).
        Node::LetTupleDestructure { names, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let tmp_idx = *next_local;
            *next_local += 1;
            chunk.emit(Op::StoreLocal(tmp_idx), line);
            for (i, name) in names.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let slot = *next_local;
                *next_local += 1;
                locals.insert(name.clone(), slot);
                chunk.emit(Op::LoadLocal(tmp_idx), line);
                let idx_const = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::Const(idx_const), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(slot), line);
            }
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(
                v,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ReturnStatement { value: None, .. } => {
            chunk.emit(Op::Return, line);
            Ok(())
        }
        Node::ExpressionStatement { expr: inner, .. } => compile_expr(
            inner,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        Node::IfStatement { .. }
        | Node::WhileStatement { .. }
        | Node::ForInStatement { .. }
        | Node::Block { .. } => compile_control_flow(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_state,
        ),
        Node::Assignment { name, value, .. } => {
            // RES-083: re-bind an existing local. Compile the RHS,
            // StoreLocal to the known slot. Unknown name is an error.
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-171a/RES-171c: `a[i] = v` and `a[i0][i1]...[iN] = v`.
        // Depth-1 lowering: LoadLocal(a), <i>, <v>, StoreIndex, StoreLocal(a).
        // Depth-N lowering: temp-local staging through compile_index_assignment.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => compile_index_assignment(
            target,
            index,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-335: `p.field = v;` where `p` is a bare Identifier.
        // Lowered as:
        //   LoadLocal(p), <v>, SetField { field }, StoreLocal(p)
        // The struct on top of the stack after `SetField` IS the
        // mutated one (VM dispatch pushes it back), so writing it
        // through `StoreLocal` commits the update. Mirrors the
        // `IndexAssignment` lowering.
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            // RES-1430: borrow target name as &str — see comment on
            // the IndexAssignment arm above.
            let local_name: &str = match target.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested field assignment (non-identifier target)",
                    ));
                }
            };
            let slot = *locals
                .get(local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.to_string()))?;
            chunk.emit(Op::LoadLocal(slot), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::SetField {
                    name_const: fname_idx,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(slot), line);
            Ok(())
        }
        // RES-break-continue: `break;` exits the innermost loop. Emit a
        // forward Jump(0) placeholder and register its PC in the
        // enclosing loop's break_patches list for back-patching once
        // the loop-exit PC is known.
        Node::Break { .. } => {
            let ls = loop_state.ok_or(CompileError::Unsupported("break outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        // RES-break-continue: `continue;` restarts the innermost loop.
        // For while loops the target is already known (the condition
        // check); for for-in loops the target is the index increment,
        // set after body compilation. Either way we emit Jump(0) and
        // let patch_loop_exits handle it.
        Node::Continue { .. } => {
            let ls = loop_state.ok_or(CompileError::Unsupported("continue outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        // RES-break-continue: `assert cond[, msg];` — evaluate the
        // condition; if falsy push the message and fail. Lowered as:
        //   <cond>
        //   JumpIfTrue(past_fail)
        //   Const(msg)
        //   AssertFail
        // past_fail:
        Node::Assert {
            condition, message, ..
        } => compile_assert(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
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
        // RES-155: `let StructName { field, other_field: local } = expr;`
        // Compile the value, store in a temp slot, then emit
        // GetField + StoreLocal for each (field_name, local_name) pair.
        Node::LetDestructureStruct { fields, value, .. } => compile_let_destructure_struct(
            fields,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-384b: `static let NAME = EXPR;` — the VM has no separate
        // statics store; compile as a regular local binding. The
        // "initialize only once" semantic is not preserved in bytecode
        // (single-execution model), but the value is accessible by name.
        Node::StaticLet { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-361: `const NAME = EXPR;` is pre-evaluated by the const_eval
        // pass before bytecode compilation. Nothing to emit at runtime.
        Node::Const { .. } => Ok(()),
        // RES-139: `live { body }` — compile the body once.
        // Retry / backoff / invariant semantics are verification-only and
        // are not emitted in the bytecode backend.
        Node::LiveBlock { body, .. } => compile_stmt(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_state,
        ),
        // Verification-only constructs: emit nothing at runtime.
        Node::Assume { .. } | Node::InvariantStatement { .. } => Ok(()),
        // Type-level / declaration-only constructs: no runtime bytecode.
        // All type information is handled at parse/typecheck time.
        Node::StructDecl { .. }
        | Node::EnumDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. }
        | Node::UnsafeBlock { .. } => Ok(()),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Shared assert-lowering logic used by both `compile_stmt` and
/// `compile_stmt_in_fn`. Emits:
///   `<cond>; JumpIfTrue(past_fail); Const(msg); AssertFail`
#[allow(clippy::too_many_arguments)]
fn compile_assert(
    condition: &Node,
    message: &Option<Box<Node>>,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        condition,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    let jt = chunk.emit(Op::JumpIfTrue(0), line);
    // Push the failure message string.
    let msg_str = if let Some(msg_node) = message {
        // If the message is a string literal we can embed it directly;
        // otherwise fall back to a generic message (complex expressions
        // aren't evaluated at compile time).
        match msg_node.as_ref() {
            Node::StringLiteral { value: s, .. } => s.clone(),
            _ => "assertion failed".to_string(),
        }
    } else {
        "assertion failed".to_string()
    };
    let msg_idx = chunk.add_string_constant(&msg_str)?;
    chunk.emit(Op::Const(msg_idx), line);
    chunk.emit(Op::AssertFail, line);
    let past_fail = chunk.code.len();
    chunk.patch_jump(jt, past_fail)?;
    Ok(())
}

/// RES-155: `let StructName { f1, f2: local } = expr;` lowering.
/// Evaluates the RHS once into a temp slot, then for each
/// `(field_name, local_name)` pair emits `LoadLocal(tmp) + GetField +
/// StoreLocal(new_slot)`. After this, `local_name` is accessible in
/// subsequent code via `LoadLocal`.
#[allow(clippy::too_many_arguments)]
fn compile_let_destructure_struct(
    fields: &[(String, String)],
    value: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        value,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let tmp_idx = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(tmp_idx), line);
    for (field_name, local_name) in fields {
        if *next_local == u16::MAX {
            return Err(CompileError::TooManyLocals);
        }
        let slot = *next_local;
        *next_local += 1;
        locals.insert(local_name.clone(), slot);
        chunk.emit(Op::LoadLocal(tmp_idx), line);
        let fname_idx = chunk.add_string_constant(field_name)?;
        chunk.emit(
            Op::GetField {
                name_const: fname_idx,
            },
            line,
        );
        chunk.emit(Op::StoreLocal(slot), line);
    }
    Ok(())
}

/// RES-083: compile if/while/block statements that share the same
/// locals environment as the enclosing scope. `Block` is flattened:
/// its inner statements are compiled inline (no new scope frame yet
/// — matches the tree walker's semantics).
///
/// `loop_state` threads break/continue patch sites from the innermost
/// enclosing loop down through Block and IfStatement arms; WhileStatement
/// and ForInStatement create a fresh inner `LoopState` and do not
/// propagate the outer one into the loop body.
#[allow(clippy::too_many_arguments)]
fn compile_control_flow(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    mut loop_state: Option<&mut LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::Block { stmts, .. } => {
            // Reborrow loop_state for each stmt sequentially.
            let mut ls = loop_state;
            for s in stmts {
                let ls_ref = ls.as_deref_mut();
                compile_stmt(
                    s,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    ls_ref,
                )?;
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
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            // JumpIfFalse to else-or-end (placeholder 0 offset)
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // consequence — pass loop_state so inner break/continue work
            compile_stmt(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_state.as_deref_mut(),
            )?;
            if let Some(alt) = alternative {
                // Unconditional jump past the else branch
                let jmp_end = chunk.emit(Op::Jump(0), line);
                // JumpIfFalse lands here (start of else)
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt(
                    alt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_state,
                )?;
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
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // Create a fresh inner LoopState — break/continue target THIS loop,
            // not any outer loop.
            let mut inner = LoopState::new(loop_start);
            compile_stmt(
                body,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                Some(&mut inner),
            )?;
            // Unconditional loop back to cond
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            // JumpIfFalse lands after the loop
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            // Patch all break sites to the exit PC.
            for p in inner.break_patches {
                chunk.patch_jump(p, end)?;
            }
            // Patch all continue sites to the loop condition check.
            for p in inner.continue_patches {
                chunk.patch_jump(p, loop_start)?;
            }
            Ok(())
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            ..
        } => compile_for_in(
            name,
            iterable,
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            /* in_fn */ false,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Compile a statement inside a `fn` body. Same as `compile_stmt`
/// except `return EXPR;` emits `ReturnFromCall` instead of `Return`
/// — a bare `return` at program scope halts the VM; one inside a
/// function returns to the caller.
///
/// `loop_state` threads break/continue patch sites from the enclosing
/// loop; same semantics as `compile_stmt`.
#[allow(clippy::too_many_arguments)]
fn compile_stmt_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    loop_state: Option<&mut LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::LetStatement { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-401: `let (a, b, c) = expr;` inside a function body.
        Node::LetTupleDestructure { names, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let tmp_idx = *next_local;
            *next_local += 1;
            chunk.emit(Op::StoreLocal(tmp_idx), line);
            for (i, name) in names.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let slot = *next_local;
                *next_local += 1;
                locals.insert(name.clone(), slot);
                chunk.emit(Op::LoadLocal(tmp_idx), line);
                let idx_const = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::Const(idx_const), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(slot), line);
            }
            Ok(())
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            compile_expr(
                v,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
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
        Node::ExpressionStatement { expr: inner, .. } => compile_expr(
            inner,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        Node::IfStatement { .. }
        | Node::WhileStatement { .. }
        | Node::ForInStatement { .. }
        | Node::Block { .. } => compile_control_flow_in_fn(
            node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_state,
        ),
        Node::Assignment { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let idx = *locals
                .get(name)
                .ok_or_else(|| CompileError::UnknownIdentifier(name.clone()))?;
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-171a/RES-171c: `a[i] = v` and `a[i0][i1]...[iN] = v`.
        // Shares the compile_index_assignment helper with compile_stmt.
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => compile_index_assignment(
            target,
            index,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-335: `p.field = v;` inside a fn body. Mirrors the
        // `compile_stmt` arm above; duplicated because the two
        // dispatchers handle `return` differently.
        Node::FieldAssignment {
            target,
            field,
            value,
            ..
        } => {
            // RES-1430: borrow target name as &str — see comment on
            // the compile_stmt IndexAssignment arm.
            let local_name: &str = match target.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => {
                    return Err(CompileError::Unsupported(
                        "nested field assignment (non-identifier target)",
                    ));
                }
            };
            let slot = *locals
                .get(local_name)
                .ok_or_else(|| CompileError::UnknownIdentifier(local_name.to_string()))?;
            chunk.emit(Op::LoadLocal(slot), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::SetField {
                    name_const: fname_idx,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(slot), line);
            Ok(())
        }
        Node::Break { .. } => {
            let ls = loop_state.ok_or(CompileError::Unsupported("break outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.break_patches.push(patch);
            Ok(())
        }
        Node::Continue { .. } => {
            let ls = loop_state.ok_or(CompileError::Unsupported("continue outside loop"))?;
            let patch = chunk.emit(Op::Jump(0), line);
            ls.continue_patches.push(patch);
            Ok(())
        }
        Node::Assert {
            condition, message, ..
        } => compile_assert(
            condition,
            message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-155: struct destructuring inside a function body.
        Node::LetDestructureStruct { fields, value, .. } => compile_let_destructure_struct(
            fields,
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-384b: `static let NAME = EXPR;` inside a fn body — same
        // treatment as the top-level arm: compile as a regular local.
        Node::StaticLet { name, value, .. } => {
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let idx = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), idx);
            chunk.emit(Op::StoreLocal(idx), line);
            Ok(())
        }
        // RES-361: const decl inside fn body — pre-evaluated, no emission.
        Node::Const { .. } => Ok(()),
        // RES-139: `live { body }` inside fn body — compile body once.
        Node::LiveBlock { body, .. } => compile_stmt_in_fn(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            loop_state,
        ),
        // Verification-only constructs: emit nothing at runtime.
        Node::Assume { .. } | Node::InvariantStatement { .. } => Ok(()),
        // Type-level / declaration-only constructs: no runtime bytecode.
        Node::StructDecl { .. }
        | Node::EnumDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::ActorDecl { .. }
        | Node::ClusterDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. }
        | Node::UnsafeBlock { .. } => Ok(()),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Same as `compile_control_flow` but routes nested statements
/// through `compile_stmt_in_fn` so `return` inside a branch emits
/// `ReturnFromCall`. This is the version used by function bodies.
#[allow(clippy::too_many_arguments)]
fn compile_control_flow_in_fn(
    node: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    mut loop_state: Option<&mut LoopState>,
) -> Result<(), CompileError> {
    match node {
        Node::Block { stmts, .. } => {
            let mut ls = loop_state;
            for s in stmts {
                let ls_ref = ls.as_deref_mut();
                compile_stmt_in_fn(
                    s,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    ls_ref,
                )?;
            }
            Ok(())
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_stmt_in_fn(
                consequence,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                loop_state.as_deref_mut(),
            )?;
            if let Some(alt) = alternative {
                let jmp_end = chunk.emit(Op::Jump(0), line);
                let else_target = chunk.code.len();
                chunk.patch_jump(jif, else_target)?;
                compile_stmt_in_fn(
                    alt,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    loop_state,
                )?;
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
            compile_expr(
                condition,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            let mut inner = LoopState::new(loop_start);
            compile_stmt_in_fn(
                body,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                Some(&mut inner),
            )?;
            let jmp = chunk.emit(Op::Jump(0), line);
            chunk.patch_jump(jmp, loop_start)?;
            let end = chunk.code.len();
            chunk.patch_jump(jif, end)?;
            for p in inner.break_patches {
                chunk.patch_jump(p, end)?;
            }
            for p in inner.continue_patches {
                chunk.patch_jump(p, loop_start)?;
            }
            Ok(())
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            ..
        } => compile_for_in(
            name,
            iterable,
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            /* in_fn */ true,
        ),
        // Type-level / declaration-only constructs: no runtime bytecode.
        Node::StructDecl { .. }
        | Node::EnumDecl { .. }
        | Node::TraitDecl { .. }
        | Node::ImplBlock { .. }
        | Node::TypeAlias { .. }
        | Node::NewtypeDecl { .. }
        | Node::RegionDecl { .. }
        | Node::ActorDecl { .. }
        | Node::ClusterDecl { .. }
        | Node::SupervisorDecl { .. }
        | Node::ModuleDecl { .. }
        | Node::Use { .. }
        | Node::UnsafeBlock { .. }
        | Node::Assume { .. }
        | Node::InvariantStatement { .. } => Ok(()),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// RES-334: compile `for NAME in ITERABLE { BODY }` to bytecode.
///
/// Models iteration on the existing `while`-loop pattern. Three
/// hidden locals carry the iterator state (the array value, the
/// integer index, and the integer length). The loop variable
/// `NAME` becomes a normal local that the body can read by
/// identifier; we re-bind it from `arr[idx]` at the top of every
/// iteration.
///
/// Today only `Value::Array` iteration is wired — strings and
/// half-open integer ranges are out of scope here (no AST node
/// for either yet) and surface as `VmError::TypeMismatch` /
/// `VmError::BuiltinCallFailed` from `LoadIndex` / `len` at run
/// time. The shape `for x in 0..10` parses inside quantifier
/// expressions only; statement position is rejected by the
/// parser before compile is reached.
///
/// Lowered shape (peephole later folds the `idx + 1` tail into a
/// single `IncLocal`):
///
/// ```text
///   <iterable>
///   StoreLocal arr_slot
///   LoadLocal arr_slot
///   CallBuiltin { "len", arity: 1 }
///   StoreLocal len_slot
///   Const 0
///   StoreLocal idx_slot
/// LOOP_START:
///   LoadLocal idx_slot
///   LoadLocal len_slot
///   Lt
///   JumpIfFalse EXIT
///   LoadLocal arr_slot
///   LoadLocal idx_slot
///   LoadIndex
///   StoreLocal name_slot
///   <body>
///   LoadLocal idx_slot
///   Const 1
///   Add
///   StoreLocal idx_slot
///   Jump LOOP_START
/// EXIT:
/// ```
#[allow(clippy::too_many_arguments)]
fn compile_for_in(
    name: &str,
    iterable: &Node,
    body: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    in_fn: bool,
) -> Result<(), CompileError> {
    // Allocate three hidden locals for the iteration state plus
    // one user-visible slot for the loop variable. Hidden slots
    // get unique synthetic names so they cannot shadow or be
    // reached by any user identifier.
    if (*next_local as usize) + 4 > u16::MAX as usize {
        return Err(CompileError::TooManyLocals);
    }
    let arr_slot = *next_local;
    *next_local += 1;
    let len_slot = *next_local;
    *next_local += 1;
    let idx_slot = *next_local;
    *next_local += 1;
    // Reserve hidden-slot keys that are not valid identifiers so
    // user code with names like "$for_arr" cannot collide. Loop
    // variable goes into the regular `locals` map so the body
    // can read it via Identifier lookup.
    let arr_key = format!("$for_arr@{}", arr_slot);
    let len_key = format!("$for_len@{}", len_slot);
    let idx_key = format!("$for_idx@{}", idx_slot);
    locals.insert(arr_key.clone(), arr_slot);
    locals.insert(len_key.clone(), len_slot);
    locals.insert(idx_key.clone(), idx_slot);
    // Loop-variable slot: shadow any outer binding for the
    // duration of this loop. The previous binding (if any) is
    // restored after the loop body so subsequent statements see
    // the original slot — matches `let`-shadowing semantics.
    let prev_name_slot = locals.get(name).copied();
    let name_slot = *next_local;
    *next_local += 1;
    locals.insert(name.to_string(), name_slot);

    // 1. Evaluate iterable, store in arr_slot.
    compile_expr(
        iterable,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::StoreLocal(arr_slot), line);

    // 2. Compute length via the canonical `len` builtin and
    //    store in len_slot. `len` handles arrays, strings, and
    //    any other iterable — the VM's LoadIndex was extended
    //    (RES-334b) to support strings so `for c in "hello"` and
    //    `for i in 0..10` (via array_range) both work uniformly.
    let len_name_const = chunk.add_string_constant("len")?;
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(
        Op::CallBuiltin {
            name_const: len_name_const,
            arity: 1,
        },
        line,
    );
    chunk.emit(Op::StoreLocal(len_slot), line);

    // 3. idx = 0
    let zero_const = chunk.add_constant(Value::Int(0))?;
    chunk.emit(Op::Const(zero_const), line);
    chunk.emit(Op::StoreLocal(idx_slot), line);

    // 4. Loop test: idx < len.
    let loop_start = chunk.code.len();
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadLocal(len_slot), line);
    chunk.emit(Op::Lt, line);
    let jif = chunk.emit(Op::JumpIfFalse(0), line);

    // 5. name = arr[idx]
    chunk.emit(Op::LoadLocal(arr_slot), line);
    chunk.emit(Op::LoadLocal(idx_slot), line);
    chunk.emit(Op::LoadIndex, line);
    chunk.emit(Op::StoreLocal(name_slot), line);

    // 6. Body. A fresh LoopState collects break/continue patch sites.
    //    `continue` in a for-in loop skips to the index increment (step 7),
    //    whose PC is not yet known — continue_patches are back-patched below.
    let mut inner = LoopState::new(0); // continue_target set after body
    if in_fn {
        compile_stmt_in_fn(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            Some(&mut inner),
        )?;
    } else {
        compile_stmt(
            body,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            Some(&mut inner),
        )?;
    }

    // 7. idx = idx + 1 (peephole folds this to IncLocal).
    // This is the `continue` target for this loop — record the PC before
    // emitting the increment so `continue` skips to here.
    let continue_target = chunk.code.len();
    chunk.emit(Op::LoadLocal(idx_slot), line);
    let one_const = chunk.add_constant(Value::Int(1))?;
    chunk.emit(Op::Const(one_const), line);
    chunk.emit(Op::Add, line);
    chunk.emit(Op::StoreLocal(idx_slot), line);

    // 8. Jump back to test.
    let jmp = chunk.emit(Op::Jump(0), line);
    chunk.patch_jump(jmp, loop_start)?;
    let end = chunk.code.len();
    chunk.patch_jump(jif, end)?;

    // Patch break → exit, continue → idx increment.
    for p in inner.break_patches {
        chunk.patch_jump(p, end)?;
    }
    for p in inner.continue_patches {
        chunk.patch_jump(p, continue_target)?;
    }

    // Restore the loop variable's outer binding. The hidden
    // iterator slots stay in `locals` so a later for-loop in
    // the same scope reuses fresh slots (next_local has already
    // moved past them).
    locals.remove(&arr_key);
    locals.remove(&len_key);
    locals.remove(&idx_key);
    if let Some(prev) = prev_name_slot {
        locals.insert(name.to_string(), prev);
    } else {
        locals.remove(name);
    }
    Ok(())
}

/// RES-171c: compile `a[i0][i1]...[iN] = v` for any nesting depth.
///
/// Extracts (root_name, indices[]) from the assignment chain, allocates
/// N-1 hidden temp locals, and emits load/mutate/writeback sequences
/// so all intermediate arrays are updated in value-semantics order.
///
/// For depth=1 this degenerates to the simple `LoadLocal / StoreIndex /
/// StoreLocal` triple (no temps needed).
#[allow(clippy::too_many_arguments)]
fn compile_index_assignment(
    target: &Node,
    outermost_index: &Node,
    value: &Node,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    // Walk target chain to collect indices in root-to-leaf order.
    let mut indices_rev: Vec<&Node> = vec![outermost_index];
    let mut cursor: &Node = target;
    let root_name = loop {
        match cursor {
            Node::Identifier { name, .. } => break name.as_str(),
            Node::IndexExpression {
                target: inner_t,
                index: inner_i,
                ..
            } => {
                indices_rev.push(inner_i.as_ref());
                cursor = inner_t.as_ref();
            }
            _ => {
                return Err(CompileError::Unsupported(
                    "non-identifier target in index assignment",
                ));
            }
        }
    };
    indices_rev.reverse();
    let indices: Vec<&Node> = indices_rev;
    let depth = indices.len(); // >= 1

    let root_slot = *locals
        .get(root_name)
        .ok_or_else(|| CompileError::UnknownIdentifier(root_name.to_string()))?;

    if depth == 1 {
        // Fast path: `a[i] = v`.
        chunk.emit(Op::LoadLocal(root_slot), line);
        compile_expr(
            indices[0],
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::StoreIndex, line);
        chunk.emit(Op::StoreLocal(root_slot), line);
        return Ok(());
    }

    // Depth >= 2: allocate N-1 temp locals.
    let n_temps = depth - 1;
    if (*next_local as usize) + n_temps > u16::MAX as usize {
        return Err(CompileError::TooManyLocals);
    }
    let temp_base = *next_local;
    *next_local += n_temps as u16;
    let temp_keys: Vec<String> = (0..n_temps)
        .map(|k| format!("$nested_idx@{}", temp_base + k as u16))
        .collect();
    for (k, key) in temp_keys.iter().enumerate() {
        locals.insert(key.clone(), temp_base + k as u16);
    }

    // Phase 1: load each intermediate level into its temp.
    // $t0 = root[i0], $t1 = $t0[i1], ..., $t(N-2) = $t(N-3)[i(N-2)]
    for (k, idx_node) in indices.iter().enumerate().take(n_temps) {
        let src_slot = if k == 0 {
            root_slot
        } else {
            temp_base + (k as u16 - 1)
        };
        chunk.emit(Op::LoadLocal(src_slot), line);
        compile_expr(
            idx_node,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::LoadIndex, line);
        chunk.emit(Op::StoreLocal(temp_base + k as u16), line);
    }

    // Phase 2: mutate the deepest temp.
    // $t(N-2)[i(N-1)] = v
    let deepest_temp = temp_base + (n_temps as u16 - 1);
    chunk.emit(Op::LoadLocal(deepest_temp), line);
    compile_expr(
        indices[depth - 1],
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    compile_expr(
        value,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    chunk.emit(Op::StoreIndex, line);
    chunk.emit(Op::StoreLocal(deepest_temp), line);

    // Phase 3: write back up the chain.
    // $t(k-1)[i(k)] = $t(k), down to root[i0] = $t0
    for k in (0..n_temps).rev() {
        let dst_slot = if k == 0 {
            root_slot
        } else {
            temp_base + (k as u16 - 1)
        };
        chunk.emit(Op::LoadLocal(dst_slot), line);
        compile_expr(
            indices[k],
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        chunk.emit(Op::LoadLocal(temp_base + k as u16), line);
        chunk.emit(Op::StoreIndex, line);
        chunk.emit(Op::StoreLocal(dst_slot), line);
    }

    // Clean up temp keys from locals map.
    for key in &temp_keys {
        locals.remove(key);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_expr(
    node: &Node,
    chunk: &mut Chunk,
    locals: &HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
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
        // RES-VM (issue #266): string + float literals. Required so
        // calls like `println("hello")` and `sin(1.5)` reach the
        // bytecode VM. The constant pool already accepts `Value::String`
        // and `Value::Float` (used today by struct/field name interning
        // and dedup); routing the literal nodes here lets builtin args
        // round-trip without touching the runtime.
        Node::StringLiteral { value: s, .. } => {
            let idx = chunk.add_string_constant(s)?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        Node::FloatLiteral { value: x, .. } => {
            let idx = chunk.add_constant(Value::Float(*x))?;
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
        } if *operator == "-" => {
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Neg, line);
            Ok(())
        }
        // RES-083: logical negation.
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "!" => {
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Not, line);
            Ok(())
        }
        // RES-083: short-circuit && desugars to `if lhs { rhs } else { false }`.
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if *operator == "&&" => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
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
        } if *operator == "||" => {
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            // Negate lhs so JumpIfFalse skips to "true" when lhs is truthy.
            chunk.emit(Op::Not, line);
            let jif = chunk.emit(Op::JumpIfFalse(0), line);
            // lhs was falsy → evaluate rhs
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
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
            compile_expr(
                left,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                right,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let op = match *operator {
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
                // Bitwise ops (integer-only; typechecker enforces operand types).
                "&" => Op::Band,
                "|" => Op::Bor,
                "^" => Op::Bxor,
                "<<" => Op::Shl,
                ">>" => Op::Shr,
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
            // RES-1419: hold the callee name as `&str` through the
            // three index lookups + `lookup_builtin` instead of
            // eagerly cloning to an owned `String`. The previous
            // shape cloned once at the top of the arm and a second
            // time at the `Value::String(callee_name.clone())` call
            // when emitting `Op::CallBuiltin` — so every builtin
            // call paid two `String` allocations. Now we only own
            // the name on the two paths that genuinely need an
            // owned value: the `Value::String` constant for the
            // CallBuiltin name and the `CompileError::UnknownFunction`
            // payload. User-fn and FFI calls (the common case) get
            // through with zero `String` clones from the callee
            // identifier. The `&str` borrows from `function.as_ref()`
            // which is alive for the whole match arm; the recursive
            // `compile_expr(arg, ...)` calls borrow disjoint
            // sub-nodes of `arguments`, so the borrow checker is
            // happy.
            // Support indirect calls: if callee is a local variable (not a named
            // fn/ffi) holding a closure, push it and emit CallClosure { arity }.
            if let Node::Identifier { name, .. } = function.as_ref() {
                let is_named =
                    fn_index.contains_key(name.as_str()) || ffi_index.contains_key(name.as_str());
                if let (false, Some(&slot)) = (is_named, locals.get(name.as_str())) {
                    chunk.emit(Op::LoadLocal(slot), line);
                    let arity = arguments.len();
                    for arg in arguments {
                        compile_expr(
                            arg,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                    }
                    if arity > u8::MAX as usize {
                        return Err(CompileError::Unsupported("too many args in indirect call"));
                    }
                    chunk.emit(Op::CallClosure { arity: arity as u8 }, line);
                    return Ok(());
                }
            }
            let callee_name: &str = match function.as_ref() {
                Node::Identifier { name, .. } => name.as_str(),
                _ => return Err(CompileError::Unsupported("indirect call on non-identifier")),
            };
            // FFI v2: foreign call takes priority over user-defined functions.
            if let Some(&idx) = ffi_index.get(callee_name) {
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                chunk.emit(Op::CallForeign(idx), line);
                return Ok(());
            }
            // User-defined function next.
            if let Some(&callee_idx) = fn_index.get(callee_name) {
                // Push args left-to-right so the VM can pop them in reverse
                // and assign to locals 0..arity in source order.
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                chunk.emit(Op::Call(callee_idx), line);
                return Ok(());
            }
            // RES-VM (issue #266): fall back to the canonical builtin
            // table. The tree walker dispatches builtins through
            // `Value::Builtin`; the bytecode VM achieves the same by
            // emitting `Op::CallBuiltin { name_const, arity }`. Limit
            // arity to u8 so the opcode stays Copy + 4 bytes; calls
            // with >255 args are rejected before any code is emitted.
            if crate::lookup_builtin(callee_name).is_some() {
                if arguments.len() > u8::MAX as usize {
                    return Err(CompileError::Unsupported("builtin call with > 255 args"));
                }
                let name_const = chunk.add_string_constant(callee_name)?;
                for arg in arguments {
                    compile_expr(
                        arg,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                }
                chunk.emit(
                    Op::CallBuiltin {
                        name_const,
                        arity: arguments.len() as u8,
                    },
                    line,
                );
                return Ok(());
            }
            Err(CompileError::UnknownFunction(callee_name.to_string()))
        }
        // RES-171a: `[a, b, c]` literal → emit each item's expression
        // left-to-right, then `Op::MakeArray { len }` which pops them
        // all into a `Value::Array`.
        Node::ArrayLiteral { items, .. } => {
            if items.len() > u16::MAX as usize {
                return Err(CompileError::Unsupported("array literal with >65535 items"));
            }
            for item in items {
                compile_expr(
                    item,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
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
        //
        // RES-407: if the typechecker's bounds-check pass discharged
        // `0 <= index < len(target)` at this exact source span, emit
        // the `LoadIndexUnchecked` variant — the runtime check is
        // redundant and the elision is what hot-loop embedded code
        // wants. Falls back to the checked op when the pass hasn't
        // run or didn't prove this site.
        Node::IndexExpression {
            target,
            index,
            span,
        } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                index,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let op = if crate::bounds_check::is_proven_site(*span) {
                Op::LoadIndexUnchecked
            } else {
                Op::LoadIndex
            };
            chunk.emit(op, line);
            Ok(())
        }
        // RES-335: `Name { f1: e1, f2: e2 }` struct literal. Lowered
        // as alternating `(field-name-const, value)` pushes followed
        // by `StructLiteral { name_const, field_count }`. Field names
        // live in the constant pool so `Op` stays `Copy`.
        Node::StructLiteral { name, fields, .. } => {
            if fields.len() > u16::MAX as usize {
                return Err(CompileError::TooManyFields(name.clone()));
            }
            let name_const = chunk.add_string_constant(name)?;
            for (field_name, field_expr) in fields {
                let fname_idx = chunk.add_string_constant(field_name)?;
                chunk.emit(Op::Const(fname_idx), line);
                compile_expr(
                    field_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
            chunk.emit(
                Op::StructLiteral {
                    name_const,
                    field_count: fields.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-335: `target.field` read → push target, emit `GetField`.
        // Nested reads (`a.b.c`) fall out of the recursion because
        // `compile_expr(target)` re-enters this arm for inner
        // `FieldAccess` nodes.
        Node::FieldAccess { target, field, .. } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let fname_idx = chunk.add_string_constant(field)?;
            chunk.emit(
                Op::GetField {
                    name_const: fname_idx,
                },
                line,
            );
            Ok(())
        }
        // RES-401: `(a, b, c)` tuple literal — compile each item left-
        // to-right then emit `MakeTuple { len }` to pack them.
        Node::TupleLiteral { items, .. } => {
            if items.len() > u16::MAX as usize {
                return Err(CompileError::Unsupported("tuple literal with >65535 items"));
            }
            for item in items {
                compile_expr(
                    item,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?;
            }
            chunk.emit(
                Op::MakeTuple {
                    len: items.len() as u16,
                },
                line,
            );
            Ok(())
        }
        // RES-401: `tuple.N` — compile the tuple, push the index as an
        // integer constant, emit `LoadIndex` (which handles both arrays
        // and tuples in the VM). The typechecker ensures `index` is
        // within the declared tuple length.
        Node::TupleIndex { tuple, index, .. } => {
            compile_expr(
                tuple,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let idx_const = chunk.add_constant(Value::Int(*index as i64))?;
            chunk.emit(Op::Const(idx_const), line);
            chunk.emit(Op::LoadIndex, line);
            Ok(())
        }
        // RES-221: interpolated string `"hello {name}!"` — lower to
        // `to_string()` calls on each expr part, then fold all parts
        // (literals are inlined as string constants) with `Op::Add`.
        //
        // Lowering: push N string values, then emit N-1 Add ops.
        // Empty interpolation (no parts) emits a single `""` constant.
        Node::InterpolatedString { parts, .. } => {
            if parts.is_empty() {
                let idx = chunk.add_string_constant("")?;
                chunk.emit(Op::Const(idx), line);
                return Ok(());
            }
            let to_string_idx = chunk.add_string_constant("to_string")?;
            for part in parts {
                match part {
                    crate::string_interp::StringPart::Literal(s) => {
                        let idx = chunk.add_string_constant(s)?;
                        chunk.emit(Op::Const(idx), line);
                    }
                    crate::string_interp::StringPart::Expr(expr) => {
                        compile_expr(
                            expr,
                            chunk,
                            locals,
                            next_local,
                            fn_index,
                            ffi_index,
                            fns,
                            next_fn_idx,
                            line,
                        )?;
                        chunk.emit(
                            Op::CallBuiltin {
                                name_const: to_string_idx,
                                arity: 1,
                            },
                            line,
                        );
                    }
                }
            }
            for _ in 1..parts.len() {
                chunk.emit(Op::Add, line);
            }
            Ok(())
        }
        // RES-163: `match scrutinee { pat => body, ... }` — lower to
        // a sequence of pattern checks followed by JumpIfFalse / Jump
        // instructions. Supports: Wildcard, Literal, Identifier,
        // Range, Or (literal branches only), Bind. Complex patterns
        // (Struct, Enum, Some/None/Ok/Err, Tuple) return Unsupported.
        Node::Match {
            scrutinee, arms, ..
        } => compile_match_expr(
            scrutinee,
            arms,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        // RES-148: `{ k1: v1, k2: v2 }` map literal. Lowered to a
        // `map_new()` call followed by N `map_insert(map, k, v)` calls.
        // All three builtins are in the BUILTINS table so the VM's
        // CallBuiltin dispatch can reach them without new opcodes.
        Node::MapLiteral { entries, .. } => {
            let map_new_idx = chunk.add_string_constant("map_new")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: map_new_idx,
                    arity: 0,
                },
                line,
            );
            if !entries.is_empty() {
                let map_insert_idx = chunk.add_string_constant("map_insert")?;
                for (k, v) in entries {
                    compile_expr(
                        k,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    compile_expr(
                        v,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    chunk.emit(
                        Op::CallBuiltin {
                            name_const: map_insert_idx,
                            arity: 3,
                        },
                        line,
                    );
                }
            }
            Ok(())
        }
        // RES-149: `#{v1, v2, v3}` set literal. Lowered to a
        // `set_new()` call followed by N `set_insert(set, item)` calls.
        Node::SetLiteral { items, .. } => {
            let set_new_idx = chunk.add_string_constant("set_new")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: set_new_idx,
                    arity: 0,
                },
                line,
            );
            if !items.is_empty() {
                let set_insert_idx = chunk.add_string_constant("set_insert")?;
                for item in items {
                    compile_expr(
                        item,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                    )?;
                    chunk.emit(
                        Op::CallBuiltin {
                            name_const: set_insert_idx,
                            arity: 2,
                        },
                        line,
                    );
                }
            }
            Ok(())
        }
        // RES-169d: `fn(params) { body }` anonymous function literal.
        // Compiles the body as a new Function entry, collects free variables
        // (capture-by-value), and emits MakeClosure.
        Node::FunctionLiteral {
            parameters, body, ..
        } => {
            if parameters.len() > u8::MAX as usize {
                return Err(CompileError::Unsupported("fn literal with >255 params"));
            }
            if *next_fn_idx == u16::MAX {
                return Err(CompileError::Unsupported("too many functions (>65535)"));
            }
            let fn_idx = *next_fn_idx;
            *next_fn_idx += 1;

            // Determine the set of free variables: identifiers in the body that
            // are not the literal's own parameters and are bound in the *outer*
            // locals map. Collect in insertion order for a deterministic capture
            // sequence (needed so LoadUpvalue(i) indices are stable).
            let param_names: std::collections::HashSet<&str> =
                parameters.iter().map(|(_, n)| n.as_str()).collect();
            let mut captured: Vec<(u16, String)> = Vec::new(); // (outer slot, name)
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            collect_free_vars(body, &param_names, locals, &mut captured, &mut seen);

            // Build the closure's local map: params at 0..arity, then upvalues
            // accessible via LoadUpvalue. The body chunk uses LoadUpvalue(i) for
            // captured names, resolved by the inner compilation below.
            let arity = parameters.len() as u8;
            let upvalue_count = captured.len();

            // Build the body chunk for the new Function entry.
            let mut fn_chunk = Chunk::with_capacity(64);
            let mut fn_locals: HashMap<String, u16> =
                HashMap::with_capacity(parameters.len().saturating_mul(2).max(8));
            let mut fn_next_local: u16 = 0;
            for (_, pname) in parameters {
                fn_locals.insert(pname.clone(), fn_next_local);
                fn_next_local += 1;
            }
            // Upvalues are accessed via Op::LoadUpvalue, not locals — we don't
            // add them to fn_locals. The body's compile_expr will see identifiers
            // missing from fn_locals and look them up as … well, currently we
            // need to NOT add them as locals so the compiled body references them
            // as upvalues. But compile_expr currently handles identifiers only
            // via locals lookup. We add a sentinel: give each captured name a
            // special "upvalue" pseudo-slot by injecting it into fn_locals with a
            // flag we then post-process. Instead, we compile the body with the
            // captured names in fn_locals, then rewrite those LoadLocal ops into
            // LoadUpvalue ops after the fact.
            //
            // Simpler approach: insert captured names into fn_locals at slots
            // >= fn_next_local (reachable area), compile, then rewrite those
            // LoadLocal(slot) ops to LoadUpvalue(upvalue_index). The upvalue
            // indices are 0-based and correspond to the capture order.
            let upvalue_base = fn_next_local; // first "upvalue" local slot
            for (i, (_, name)) in captured.iter().enumerate() {
                fn_locals.insert(name.clone(), upvalue_base + i as u16);
            }

            // Compile the body statements.
            let inner_stmts = match body.as_ref() {
                Node::Block { stmts: b, .. } => b.as_slice(),
                single => std::slice::from_ref(single),
            };
            for stmt in inner_stmts {
                let stmt_line = node_line(stmt).unwrap_or(line);
                compile_stmt_in_fn(
                    stmt,
                    &mut fn_chunk,
                    &mut fn_locals,
                    &mut fn_next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    stmt_line,
                    None,
                )?;
            }
            fn_chunk.emit(Op::ReturnFromCall, 0);

            // Rewrite LoadLocal(upvalue_base + i) → LoadUpvalue(i).
            // Any slot in [upvalue_base, upvalue_base + upvalue_count) was
            // injected for a capture. Rewrite those slots in-place.
            for op in &mut fn_chunk.code {
                // `if let … { if … }` form is intentional: stable Rust doesn't
                // have let_chains, so suppress the collapsible_if lint here.
                #[allow(clippy::collapsible_if)]
                if let Op::LoadLocal(slot) = op {
                    if *slot >= upvalue_base
                        && (*slot as usize) < upvalue_base as usize + upvalue_count
                    {
                        *op = Op::LoadUpvalue(*slot - upvalue_base);
                    }
                }
            }

            let local_count = fn_next_local;
            // Insert at fn_idx (pre-allocated index). fns may have grown via
            // nested FunctionLiterals; we need to push a placeholder then
            // overwrite it, OR we always push at end (and fn_idx == fns.len()
            // at the time we called *next_fn_idx += 1). Since nested closures
            // also increment next_fn_idx, fn_idx may not equal fns.len() by
            // the time we reach here. Use a placeholder-then-overwrite strategy:
            // extend fns to at least fn_idx+1 with placeholders.
            while fns.len() <= fn_idx as usize {
                fns.push(Function {
                    name: "<closure_placeholder>".into(),
                    arity: 0,
                    chunk: Chunk::with_capacity(0),
                    local_count: 0,
                });
            }
            fns[fn_idx as usize] = Function {
                name: "<closure>".into(),
                arity,
                chunk: fn_chunk,
                local_count,
            };

            // Emit: push each captured value onto the stack, then MakeClosure.
            for (outer_slot, _) in &captured {
                chunk.emit(Op::LoadLocal(*outer_slot), line);
            }
            chunk.emit(
                Op::MakeClosure {
                    fn_idx,
                    upvalue_count: upvalue_count as u8,
                },
                line,
            );
            Ok(())
        }
        // RES-152: `b"..."` bytes literal — stored as a Value::Bytes constant.
        Node::BytesLiteral { value, .. } => {
            let idx = chunk.add_constant(Value::Bytes(value.clone()))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-291: `lo..hi` / `lo..=hi` range expression.
        // Lowered to `array_range(lo, hi)` for exclusive ranges, or
        // `array_range(lo, hi + 1)` for inclusive ranges (emit hi, Const(1), Add).
        Node::Range {
            lo, hi, inclusive, ..
        } => {
            compile_expr(
                lo,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            compile_expr(
                hi,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            if *inclusive {
                // hi_incl = hi + 1
                let one_idx = chunk.add_constant(Value::Int(1))?;
                chunk.emit(Op::Const(one_idx), line);
                chunk.emit(Op::Add, line);
            }
            let name_idx = chunk.add_string_constant("array_range")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: name_idx,
                    arity: 2,
                },
                line,
            );
            Ok(())
        }
        // RES-921: `target[lo..hi]` / `target[lo..=hi]` slice expression.
        // Lowered to `array_slice(target, lo, hi, inclusive)`.
        // `lo = None` is represented as `Value::Int(0)`;
        // `hi = None` is represented as `Value::Int(-1)` (sentinel: end of array).
        Node::Slice {
            target,
            lo,
            hi,
            inclusive,
            ..
        } => {
            compile_expr(
                target,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            match lo {
                Some(lo_expr) => compile_expr(
                    lo_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?,
                None => {
                    let idx = chunk.add_constant(Value::Int(0))?;
                    chunk.emit(Op::Const(idx), line);
                }
            }
            match hi {
                Some(hi_expr) => compile_expr(
                    hi_expr,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                )?,
                None => {
                    // -1 sentinel = "up to end of array"
                    let idx = chunk.add_constant(Value::Int(-1))?;
                    chunk.emit(Op::Const(idx), line);
                }
            }
            let incl_idx = chunk.add_constant(Value::Bool(*inclusive))?;
            chunk.emit(Op::Const(incl_idx), line);
            let name_idx = chunk.add_string_constant("array_slice")?;
            chunk.emit(
                Op::CallBuiltin {
                    name_const: name_idx,
                    arity: 4,
                },
                line,
            );
            Ok(())
        }
        // RES-1857/RES-duration: DurationLiteral is a nanoseconds constant.
        // The nanos value is already computed by the parser; emit it as an Int.
        Node::DurationLiteral { nanos, .. } => {
            let idx = chunk.add_constant(Value::Int(*nanos as i64))?;
            chunk.emit(Op::Const(idx), line);
            Ok(())
        }
        // RES-newtypes: NewtypeConstruct wraps a value in a one-field struct.
        // The interpreter creates `Struct { name, fields: [("__value", inner)] }`;
        // we replicate that by emitting a string-const for "__value", compiling
        // the inner expression, then emitting StructLiteral with field_count=1.
        Node::NewtypeConstruct {
            type_name, value, ..
        } => {
            let name_const = chunk.add_string_constant(type_name)?;
            let field_name_idx = chunk.add_string_constant("__value")?;
            chunk.emit(Op::Const(field_name_idx), line);
            compile_expr(
                value,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(
                Op::StructLiteral {
                    name_const,
                    field_count: 1,
                },
                line,
            );
            Ok(())
        }
        // RES-375: TryExpression (`expr?`) — compile the inner expression,
        // then emit TryUnwrap which either leaves the unwrapped value on the
        // stack or triggers an early return from the current function.
        Node::TryExpression { expr: inner, .. } => {
            compile_expr(
                inner,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::TryUnwrap, line);
            Ok(())
        }
        // RES-325: NamedArg — the name is a type-check annotation only;
        // for bytecode purposes just compile the value.
        Node::NamedArg { value, .. } => compile_expr(
            value,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        ),
        other => Err(CompileError::Unsupported(node_kind(other))),
    }
}

/// Walk `node` collecting identifiers that are free in the expression (not
/// in `param_names`) and bound in `outer_locals`. Results go into `out` in
/// first-seen order; `seen` tracks which names we've already added.
fn collect_free_vars(
    node: &Node,
    param_names: &std::collections::HashSet<&str>,
    outer_locals: &HashMap<String, u16>,
    out: &mut Vec<(u16, String)>,
    seen: &mut std::collections::HashSet<String>,
) {
    match node {
        Node::Identifier { name, .. }
            if !param_names.contains(name.as_str())
                && !seen.contains(name)
                && outer_locals.contains_key(name) =>
        {
            let slot = outer_locals[name];
            seen.insert(name.clone());
            out.push((slot, name.clone()));
        }
        Node::Identifier { .. } => {}
        // Recurse into all child nodes.
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_free_vars(s, param_names, outer_locals, out, seen);
            }
        }
        Node::LetStatement { value, .. } => {
            collect_free_vars(value, param_names, outer_locals, out, seen);
        }
        Node::InfixExpression { left, right, .. } => {
            collect_free_vars(left, param_names, outer_locals, out, seen);
            collect_free_vars(right, param_names, outer_locals, out, seen);
        }
        Node::PrefixExpression { right, .. } => {
            collect_free_vars(right, param_names, outer_locals, out, seen);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_free_vars(function, param_names, outer_locals, out, seen);
            for a in arguments {
                collect_free_vars(a, param_names, outer_locals, out, seen);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_free_vars(condition, param_names, outer_locals, out, seen);
            collect_free_vars(consequence, param_names, outer_locals, out, seen);
            if let Some(alt) = alternative {
                collect_free_vars(alt, param_names, outer_locals, out, seen);
            }
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_free_vars(v, param_names, outer_locals, out, seen);
        }
        Node::ReturnStatement { .. } => {}
        Node::ExpressionStatement { expr, .. } => {
            collect_free_vars(expr, param_names, outer_locals, out, seen);
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_free_vars(condition, param_names, outer_locals, out, seen);
            collect_free_vars(body, param_names, outer_locals, out, seen);
        }
        // Leaf nodes (literals, etc.) have no free vars.
        _ => {}
    }
}

// ── Match expression lowering ─────────────────────────────────────────────────

/// Compile a `match` expression. The scrutinee is evaluated once and
/// stored in a hidden temp local; each arm is compiled as a
/// pattern-check + optional-guard + body sequence with jump routing.
#[allow(clippy::too_many_arguments)]
fn compile_match_expr(
    scrutinee: &Node,
    arms: &[(crate::Pattern, Option<Node>, Node)],
    chunk: &mut Chunk,
    locals: &HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
) -> Result<(), CompileError> {
    compile_expr(
        scrutinee,
        chunk,
        locals,
        next_local,
        fn_index,
        ffi_index,
        fns,
        next_fn_idx,
        line,
    )?;
    if *next_local == u16::MAX {
        return Err(CompileError::TooManyLocals);
    }
    let scrutinee_slot = *next_local;
    *next_local += 1;
    chunk.emit(Op::StoreLocal(scrutinee_slot), line);

    // RES-1922: exact upper bound — every arm pushes one entry below
    // (its "jump past the whole match" patch). Skips the default
    // 0→4→8→16 grow chain for matches with ≥ 4 arms (common shape
    // for enum-exhaustive matches over Result / Option / custom sum
    // types). Same shape as RES-1800 / RES-1762 / RES-1796 pre-sizes.
    let mut after_match_patches: Vec<usize> = Vec::with_capacity(arms.len());

    for (pattern, guard, body) in arms {
        // Each arm gets its own mutable locals copy so bindings don't
        // leak across arms. The clone is cheap (typically ≤ 16 entries).
        let mut arm_locals = locals.clone();
        let next_local_snap = *next_local;

        let mut next_arm_patches: Vec<usize> = Vec::new();
        compile_pattern_check(
            pattern,
            scrutinee_slot,
            chunk,
            &mut arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
            &mut next_arm_patches,
        )?;

        if let Some(guard_expr) = guard {
            compile_expr(
                guard_expr,
                chunk,
                &arm_locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }

        compile_expr(
            body,
            chunk,
            &arm_locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;

        let after_p = chunk.emit(Op::Jump(0), line);
        after_match_patches.push(after_p);

        let next_arm_pc = chunk.code.len();
        for p in next_arm_patches {
            chunk.patch_jump(p, next_arm_pc)?;
        }

        // Reclaim temp slots used by this arm's bindings.
        *next_local = next_local_snap;
    }

    // Fallthrough (no arm matched) → Void.
    let void_idx = chunk.add_constant(Value::Void)?;
    chunk.emit(Op::Const(void_idx), line);

    let after_match_pc = chunk.code.len();
    for p in after_match_patches {
        chunk.patch_jump(p, after_match_pc)?;
    }
    Ok(())
}

/// Emit code that checks whether the current scrutinee (in `scrutinee_slot`)
/// matches `pattern`. On failure, a `JumpIfFalse(0)` placeholder is appended
/// to `next_arm_patches` (caller patches it to the next arm). On success, any
/// name bindings are added to `locals`.
///
/// Supported: Wildcard, Literal, Identifier, Range, Or (literal branches),
/// Bind(name, inner). Complex structural patterns return `Unsupported`.
#[allow(clippy::too_many_arguments)]
fn compile_pattern_check(
    pattern: &crate::Pattern,
    scrutinee_slot: u16,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    line: u32,
    next_arm_patches: &mut Vec<usize>,
) -> Result<(), CompileError> {
    use crate::Pattern;
    match pattern {
        Pattern::Wildcard => {
            // Always matches — no code.
        }
        Pattern::Literal(lit_node) => {
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            compile_expr(
                lit_node,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
            )?;
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }
        Pattern::Identifier(name) => {
            // Bind the scrutinee value to `name`; always matches.
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let slot = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), slot);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::StoreLocal(slot), line);
        }
        Pattern::Range { lo, hi, inclusive } => {
            // lo <= scrutinee
            let lo_idx = chunk.add_constant(Value::Int(*lo))?;
            chunk.emit(Op::Const(lo_idx), line);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::Le, line);
            let p1 = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p1);
            // scrutinee <= hi  (or < hi)
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            let hi_idx = chunk.add_constant(Value::Int(*hi))?;
            chunk.emit(Op::Const(hi_idx), line);
            if *inclusive {
                chunk.emit(Op::Le, line);
            } else {
                chunk.emit(Op::Lt, line);
            }
            let p2 = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p2);
        }
        Pattern::Bind(name, inner) => {
            // Check inner pattern first; then bind `name` if it matched.
            compile_pattern_check(
                inner,
                scrutinee_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let slot = *next_local;
            *next_local += 1;
            locals.insert(name.clone(), slot);
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(Op::StoreLocal(slot), line);
        }
        Pattern::Or(branches) => {
            // Only support Or over Literal / Wildcard branches (no bindings).
            if branches.iter().any(pattern_has_bindings) {
                return Err(CompileError::Unsupported(
                    "Or pattern with identifier bindings",
                ));
            }
            // For each branch except the last: check; on match, jump to
            // or_matched. For the last: check; on fail, fall to next_arm.
            let mut or_matched_patches: Vec<usize> = Vec::new();
            for (i, branch) in branches.iter().enumerate() {
                let is_last = i == branches.len() - 1;
                if is_last {
                    // Last branch: normal "fail → next arm" check.
                    compile_pattern_check(
                        branch,
                        scrutinee_slot,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                        next_arm_patches,
                    )?;
                } else {
                    // Non-last: emit check that jumps to or_matched on
                    // success. We invert: collect a "fail" patch from the
                    // check, emit Jump(or_matched), then patch the fail
                    // to skip the jump (i.e., continue to the next branch).
                    let mut branch_fail: Vec<usize> = Vec::new();
                    compile_pattern_check(
                        branch,
                        scrutinee_slot,
                        chunk,
                        locals,
                        next_local,
                        fn_index,
                        ffi_index,
                        fns,
                        next_fn_idx,
                        line,
                        &mut branch_fail,
                    )?;
                    // Branch matched if no JumpIfFalse was taken.
                    let matched_p = chunk.emit(Op::Jump(0), line);
                    or_matched_patches.push(matched_p);
                    // Patch branch_fail to here (next branch check).
                    let next_branch_pc = chunk.code.len();
                    for p in branch_fail {
                        chunk.patch_jump(p, next_branch_pc)?;
                    }
                }
            }
            // or_matched: all or_matched_patches land here.
            let or_matched_pc = chunk.code.len();
            for p in or_matched_patches {
                chunk.patch_jump(p, or_matched_pc)?;
            }
        }
        // RES-375: `None` — checks that scrutinee is an absent Option.
        Pattern::None => {
            let n = chunk.add_string_constant("is_none")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
        }
        // RES-375: `Some(inner)` — checks is_some, then extracts and matches inner.
        Pattern::Some(inner_pat) => {
            // 1. is_some(scrutinee) check.
            let is_some_n = chunk.add_string_constant("is_some")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_some_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            // 2. Extract inner: option_unwrap(scrutinee).
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uw_n = chunk.add_string_constant("option_unwrap")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uw_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            // 3. Check inner pattern.
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-923: `Ok(inner)` — checks is_ok, then extracts and matches inner.
        Pattern::Ok(inner_pat) => {
            let is_ok_n = chunk.add_string_constant("is_ok")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_ok_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uw_n = chunk.add_string_constant("unwrap")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uw_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-923: `Err(inner)` — checks is_err, then extracts and matches inner.
        Pattern::Err(inner_pat) => {
            let is_err_n = chunk.add_string_constant("is_err")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_err_n,
                    arity: 1,
                },
                line,
            );
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            if *next_local == u16::MAX {
                return Err(CompileError::TooManyLocals);
            }
            let inner_slot = *next_local;
            *next_local += 1;
            let uwe_n = chunk.add_string_constant("unwrap_err")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: uwe_n,
                    arity: 1,
                },
                line,
            );
            chunk.emit(Op::StoreLocal(inner_slot), line);
            compile_pattern_check(
                inner_pat,
                inner_slot,
                chunk,
                locals,
                next_local,
                fn_index,
                ffi_index,
                fns,
                next_fn_idx,
                line,
                next_arm_patches,
            )?;
        }
        // RES-932: `(p0, p1, ...)` — checks type, checks length, checks elements.
        Pattern::Tuple(sub_pats) => {
            // 1. Confirm the scrutinee is actually a Tuple (not an Array).
            let is_tup_n = chunk.add_string_constant("is_tuple")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: is_tup_n,
                    arity: 1,
                },
                line,
            );
            let p_type = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p_type);
            // 2. Check length via `len` builtin.
            let len_n = chunk.add_string_constant("len")?;
            chunk.emit(Op::LoadLocal(scrutinee_slot), line);
            chunk.emit(
                Op::CallBuiltin {
                    name_const: len_n,
                    arity: 1,
                },
                line,
            );
            let expected_len = chunk.add_constant(Value::Int(sub_pats.len() as i64))?;
            chunk.emit(Op::Const(expected_len), line);
            chunk.emit(Op::Eq, line);
            let p = chunk.emit(Op::JumpIfFalse(0), line);
            next_arm_patches.push(p);
            // Check each element.
            for (i, sub_pat) in sub_pats.iter().enumerate() {
                if *next_local == u16::MAX {
                    return Err(CompileError::TooManyLocals);
                }
                let elem_slot = *next_local;
                *next_local += 1;
                let i_idx = chunk.add_constant(Value::Int(i as i64))?;
                chunk.emit(Op::LoadLocal(scrutinee_slot), line);
                chunk.emit(Op::Const(i_idx), line);
                chunk.emit(Op::LoadIndex, line);
                chunk.emit(Op::StoreLocal(elem_slot), line);
                compile_pattern_check(
                    sub_pat,
                    elem_slot,
                    chunk,
                    locals,
                    next_local,
                    fn_index,
                    ffi_index,
                    fns,
                    next_fn_idx,
                    line,
                    next_arm_patches,
                )?;
            }
        }
        _ => {
            return Err(CompileError::Unsupported("complex match pattern"));
        }
    }
    Ok(())
}

/// Returns true if the pattern introduces any identifier bindings.
fn pattern_has_bindings(p: &crate::Pattern) -> bool {
    use crate::Pattern;
    match p {
        Pattern::Identifier(_) | Pattern::Bind(_, _) => true,
        Pattern::Or(branches) => branches.iter().any(pattern_has_bindings),
        Pattern::Wildcard | Pattern::Literal(_) | Pattern::Range { .. } | Pattern::None => false,
        Pattern::Struct { fields, .. } => fields.iter().any(|(_, p)| pattern_has_bindings(p)),
        Pattern::Tuple(ps) => ps.iter().any(pattern_has_bindings),
        Pattern::TupleStruct { fields, .. } => fields.iter().any(pattern_has_bindings),
        Pattern::Some(inner) | Pattern::Ok(inner) | Pattern::Err(inner) => {
            pattern_has_bindings(inner)
        }
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => false,
            crate::EnumPatternPayload::Named(fields) => {
                fields.iter().any(|(_, p)| pattern_has_bindings(p))
            }
            crate::EnumPatternPayload::Tuple(ps) => ps.iter().any(pattern_has_bindings),
        },
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
        // Statement variants (RES-079, RES-361).
        Node::LetStatement { span, .. }
        | Node::StaticLet { span, .. }
        | Node::Const { span, .. }
        | Node::Assignment { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::Break { span, .. }
        | Node::Continue { span, .. }
        | Node::BreakLabel { span, .. }
        | Node::ContinueLabel { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::Slice { span, .. } => span.start.line as u32,

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
        | Node::InvariantStatement { span, .. }
        | Node::Match { span, .. }
        | Node::StructDecl { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::ImplBlock { span, .. }
        | Node::TypeAlias { span, .. }
        | Node::RegionDecl { span, .. }
        | Node::Actor { span, .. }
        | Node::ActorDecl { span, .. }
        | Node::ClusterDecl { span, .. }
        | Node::FunctionLiteral { span, .. }
        | Node::TryCatch { span, .. }
        | Node::Quantifier { span, .. }
        | Node::SupervisorDecl { span, .. } => span.start.line as u32,

        // RES-291: integer range expression. Only emitted from the
        // tree-walker frontend today; bytecode lowering treats it as
        // unsupported.
        Node::Range { span, .. } => span.start.line as u32,

        // RES-142: duration literal carries the span of its integer
        // part; only emitted inside live-clause position so it
        // shouldn't round-trip through the compiler, but match it
        // anyway to keep the pattern exhaustive.
        Node::DurationLiteral { span, .. } => span.start.line as u32,

        // Program is wrapped in Spanned<Node> at the call site, not
        // inside the Node enum itself.
        Node::Program(_) => 0,

        // RES-325: NamedArg carries the span of its `name:` label.
        Node::NamedArg { span, .. } => span.start.line as u32,
        // RES-221: interpolated string carries the opening quote's span.
        Node::InterpolatedString { span, .. } => span.start.line as u32,

        // RES-324: module declaration; span at the `mod` keyword.
        Node::ModuleDecl { span, .. } => span.start.line as u32,

        // RES-319: newtype nodes carry a span.
        Node::NewtypeDecl { span, .. } => span.start.line as u32,
        Node::NewtypeConstruct { span, .. } => span.start.line as u32,
        // RES-401: tuples carry their own spans.
        Node::TupleLiteral { span, .. } => span.start.line as u32,
        Node::TupleIndex { span, .. } => span.start.line as u32,
        Node::LetTupleDestructure { span, .. } => span.start.line as u32,
        // RES-290: trait declarations carry a span.
        Node::TraitDecl { span, .. } => span.start.line as u32,
        // RES-400 PR 1: enum declarations carry a span.
        Node::EnumDecl { span, .. } => span.start.line as u32,
        // RES-406: unsafe block carries the keyword's span.
        Node::UnsafeBlock { span, .. } => span.start.line as u32,
        // RES-395: region type-param node — carries its declaration span.
        Node::RegionParam { span, .. } => span.start.line as u32,
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
        Node::InvariantStatement { .. } => "InvariantStatement",
        Node::Block { .. } => "Block",
        Node::LetStatement { .. } => "LetStatement",
        Node::StaticLet { .. } => "StaticLet",
        Node::Const { .. } => "Const",
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
        Node::StructLiteral { .. } => "StructLiteral",
        Node::FieldAccess { .. } => "FieldAccess",
        Node::FieldAssignment { .. } => "FieldAssignment",
        Node::BytesLiteral { .. } => "BytesLiteral",
        Node::Range { .. } => "Range",
        Node::Slice { .. } => "Slice",
        Node::LetTupleDestructure { .. } => "LetTupleDestructure",
        Node::LetDestructureStruct { .. } => "LetDestructureStruct",
        Node::TupleLiteral { .. } => "TupleLiteral",
        Node::TupleIndex { .. } => "TupleIndex",
        Node::MapLiteral { .. } => "MapLiteral",
        Node::SetLiteral { .. } => "SetLiteral",
        Node::Match { .. } => "Match",
        Node::FunctionLiteral { .. } => "FunctionLiteral",
        Node::InterpolatedString { .. } => "InterpolatedString",
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
    // RES-1581: fuse the two-pass collect-then-rewrite into a single
    // walk. The intermediate `Vec<usize>` of positions was unnecessary
    // — rewriting in place doesn't break the next iteration's read
    // because the new `Op::Return` tombstone at `i+1` cannot itself
    // match `Op::Call(own_fn_idx)` at any later i. Drops the Vec
    // allocation and halves the linear scans.
    for i in 0..len - 1 {
        if chunk.code[i] == Op::Call(own_fn_idx) && chunk.code[i + 1] == Op::ReturnFromCall {
            // Replace the Call with TailCall; mark the ReturnFromCall
            // dead by overwriting with a no-op Return. The VM never
            // reaches it because TailCall resets pc, but leaving a
            // valid opcode keeps the chunk well-formed for the
            // disassembler and any future static analyses.
            chunk.code[i] = Op::TailCall(own_fn_idx);
            chunk.code[i + 1] = Op::Return; // unreachable tombstone
        }
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
        // RES-1579: pre-size `entries` to the StructDecl count. Same
        // shape as RES-1461 (fn_index), RES-1575 (locals), RES-1577
        // (ffi_index) — counting once is cheap, avoids the default
        // rehash chain as the registry grows.
        let struct_count = stmts
            .iter()
            .filter(|s| matches!(&s.node, Node::StructDecl { .. }))
            .count();
        let mut entries: HashMap<String, StructRegistryEntry> =
            HashMap::with_capacity(struct_count);
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
        // RES-163: basic match (literal/wildcard arms) is now compiled.
        // Structural patterns (struct destructuring) are still unsupported.
        // Use a struct pattern as the "unsupported construct" canary until
        // struct-pattern lowering ships.
        let p = parse_one(
            r#"struct Point { int x, int y }
            fn classify(Point p) -> int {
                return match p {
                    Point { x: 0, y: 0 } => 1,
                    _ => 0,
                };
            }"#,
        );
        let err = compile(&p).unwrap_err();
        assert!(matches!(err, CompileError::Unsupported(_)), "{:?}", err);
    }

    // ---------- RES-334: for-in lowering ----------

    /// `for x in arr { ... }` no longer reports `Unsupported`. The
    /// chunk should compile cleanly and the loop variable's slot
    /// should be readable inside the body.
    #[test]
    fn res334_for_in_array_compiles() {
        let p = parse_one(
            r#"
                let arr = [1, 2, 3];
                let total = 0;
                for x in arr {
                    total = total + x;
                }
            "#,
        );
        let prog = compile(&p).expect("for-in must compile");
        // Loop body must read the loop variable: `LoadIndex` produces
        // it and `StoreLocal` commits it; then a `LoadLocal` of that
        // same slot must follow inside the body.
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::LoadIndex)),
            "expected LoadIndex in for-in body: {:?}",
            prog.main.code
        );
    }

    /// The lowered shape includes a `len` builtin call to compute
    /// the iteration bound. Verify the constant pool carries the
    /// builtin name and the chunk emits `CallBuiltin`.
    #[test]
    fn res334_for_in_uses_len_builtin() {
        let p = parse_one(
            r#"
                let arr = [10, 20];
                for x in arr { let y = x; }
            "#,
        );
        let prog = compile(&p).expect("for-in compiles");
        let mut saw_len = false;
        for op in &prog.main.code {
            if let Op::CallBuiltin { name_const, arity } = op {
                let s = match prog.main.constants.get(*name_const as usize) {
                    Some(Value::String(s)) => s.clone(),
                    _ => continue,
                };
                if s == "len" {
                    assert_eq!(*arity, 1, "len call must have arity 1");
                    saw_len = true;
                }
            }
        }
        assert!(
            saw_len,
            "expected a CallBuiltin(len, 1) for the iteration bound"
        );
    }

    /// for-in must include a back-edge `Jump` to the loop test and a
    /// forward `JumpIfFalse` exiting the loop, mirroring `while`.
    #[test]
    fn res334_for_in_emits_back_edge_and_exit_jump() {
        let p = parse_one(
            r#"
                let arr = [1];
                for x in arr { let y = x; }
            "#,
        );
        let prog = compile(&p).expect("for-in compiles");
        let has_back_edge = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::Jump(off) if *off < 0));
        let has_exit = prog
            .main
            .code
            .iter()
            .any(|op| matches!(op, Op::JumpIfFalse(off) if *off > 0));
        assert!(
            has_back_edge,
            "expected a negative-offset Jump (back-edge): {:?}",
            prog.main.code
        );
        assert!(
            has_exit,
            "expected a positive-offset JumpIfFalse (exit): {:?}",
            prog.main.code
        );
    }

    /// for-in inside a function body must compile through the
    /// `compile_stmt_in_fn` dispatcher so a `return` in the body
    /// emits `ReturnFromCall`, not `Return`.
    #[test]
    fn res334_for_in_in_fn_body_compiles_with_return_from_call() {
        let p = parse_one(
            r#"
                fn first(int dummy) -> int {
                    let xs = [1, 2, 3];
                    for x in xs {
                        return x;
                    }
                    return -1;
                }
            "#,
        );
        let prog = compile(&p).expect("for-in inside fn compiles");
        let f = &prog.functions[0];
        assert!(
            f.chunk
                .code
                .iter()
                .any(|op| matches!(op, Op::ReturnFromCall)),
            "expected ReturnFromCall inside fn body: {:?}",
            f.chunk.code
        );
        // No bare `Op::Return` should appear in a fn body.
        assert!(
            !f.chunk.code.iter().any(|op| matches!(op, Op::Return)),
            "fn body must not emit Op::Return (halts VM); got {:?}",
            f.chunk.code
        );
    }

    /// Nested for-in must allocate non-overlapping iteration-state
    /// slots so the outer loop's index isn't clobbered by the
    /// inner loop.
    #[test]
    fn res334_nested_for_in_compiles() {
        let p = parse_one(
            r#"
                let outer = [[1, 2], [3]];
                let total = 0;
                for row in outer {
                    for x in row {
                        total = total + x;
                    }
                }
            "#,
        );
        let prog = compile(&p).expect("nested for-in compiles");
        // Two distinct StoreLocal targets must be initialised to 0
        // (the inner and outer index slots). The pattern looks for
        // `Const(<int 0>); StoreLocal(s)` pairs.
        let mut zero_init_slots: Vec<u16> = Vec::new();
        let mut prev: Option<&Op> = None;
        for op in &prog.main.code {
            if let Some(Op::Const(c)) = prev
                && let Op::StoreLocal(slot) = op
                && matches!(prog.main.constants.get(*c as usize), Some(Value::Int(0)))
            {
                zero_init_slots.push(*slot);
            }
            prev = Some(op);
        }
        assert!(
            zero_init_slots.len() >= 2,
            "expected at least two zero-initialised index slots in nested for-in: got {:?}",
            zero_init_slots
        );
    }

    // ---------- RES-334b: string + range iteration ----------

    #[test]
    fn res334b_for_in_string_compiles() {
        // `for c in "hi"` must compile without errors.
        let p = parse_one(r#"let s = "hi"; let n = 0; for c in s { n = n + 1; } return n;"#);
        compile(&p).expect("for-in over string compiles");
    }

    #[test]
    fn res334b_for_in_range_compiles() {
        // `for i in 0..3` must compile — the range is lowered to
        // array_range(0, 3) by compile_expr.
        let p = parse_one("let n = 0; for i in 0..3 { n = n + i; } return n;");
        compile(&p).expect("for-in over range compiles");
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

    /// RES-VM (issue #266): `println("hi")` lowers to a `CallBuiltin`
    /// op (not `Call`, which is for user functions). The constant pool
    /// holds the builtin's name as a `Value::String`; arity is the
    /// argument count.
    #[test]
    fn compile_println_emits_call_builtin() {
        let p = parse_one("println(\"hi\");");
        let prog = compile(&p).unwrap();
        // Find the CallBuiltin op and verify its constant resolves
        // to the builtin name.
        let mut found = false;
        for op in &prog.main.code {
            if let Op::CallBuiltin { name_const, arity } = op {
                let name = match prog.main.constants.get(*name_const as usize) {
                    Some(Value::String(s)) => s.clone(),
                    other => panic!("expected Value::String at name_const, got {:?}", other),
                };
                assert_eq!(name, "println");
                assert_eq!(*arity, 1);
                found = true;
            }
        }
        assert!(
            found,
            "expected a CallBuiltin op in main.code: {:?}",
            prog.main.code
        );
    }

    /// RES-VM (issue #266): a user-defined function with the same
    /// name as a builtin shadows the builtin. Compile path picks the
    /// user fn (Call), not CallBuiltin — mirrors the tree walker's
    /// lookup order where the user binding wins.
    #[test]
    fn compile_user_fn_shadows_builtin() {
        let p = parse_one("fn println() { return 1; } println();");
        let prog = compile(&p).unwrap();
        assert!(
            prog.main.code.iter().any(|op| matches!(op, Op::Call(_))),
            "expected Call (user fn) in main.code: {:?}",
            prog.main.code
        );
        assert!(
            !prog
                .main
                .code
                .iter()
                .any(|op| matches!(op, Op::CallBuiltin { .. })),
            "user fn must shadow builtin; got: {:?}",
            prog.main.code
        );
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

    // ---------- RES-407: bounds-check elision ----------

    use std::sync::Mutex;
    static BOUNDS_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Walk every chunk in `prog` (main + all user fns) and count
    /// occurrences of `LoadIndex` and `LoadIndexUnchecked`.
    fn count_load_index_ops(prog: &Program) -> (usize, usize) {
        let chunks = std::iter::once(&prog.main).chain(prog.functions.iter().map(|f| &f.chunk));
        let mut checked = 0usize;
        let mut unchecked = 0usize;
        for c in chunks {
            for op in &c.code {
                match op {
                    Op::LoadIndex => checked += 1,
                    Op::LoadIndexUnchecked => unchecked += 1,
                    _ => {}
                }
            }
        }
        (checked, unchecked)
    }

    #[test]
    fn res407_proven_literal_index_emits_unchecked_load() {
        // `lock()` may poison if a sibling test panicked; recover the
        // guard so this test doesn't transitively fail.
        let _g = BOUNDS_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let src = r#"
fn main() {
    let xs = [10, 20, 30];
    let y = xs[1];
}
main();
"#;
        let program = parse_one(src);
        // Pass needs to run before compile so the proven-sites set is
        // populated. The compiler reads it via thread-local.
        crate::bounds_check::check_array_bounds(&program, "<test>").unwrap();
        let prog = compile(&program).expect("compiles");
        let (checked, unchecked) = count_load_index_ops(&prog);
        assert_eq!(
            unchecked, 1,
            "expected one LoadIndexUnchecked for proven xs[1] (checked={})",
            checked
        );
        assert_eq!(
            checked, 0,
            "expected no checked LoadIndex (unchecked={})",
            unchecked
        );
    }

    #[test]
    fn res407_unprovable_index_keeps_checked_load() {
        let _g = BOUNDS_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        // `i` is a free parameter — bounds_check can't prove it.
        let src = r#"
fn get(int i) -> int {
    let xs = [1, 2, 3];
    return xs[i];
}
"#;
        let program = parse_one(src);
        crate::bounds_check::check_array_bounds(&program, "<test>").unwrap();
        let prog = compile(&program).expect("compiles");
        let (checked, unchecked) = count_load_index_ops(&prog);
        assert_eq!(
            unchecked, 0,
            "expected no LoadIndexUnchecked for dynamic xs[i] (checked={})",
            checked
        );
        assert!(
            checked >= 1,
            "expected at least one checked LoadIndex for dynamic xs[i]"
        );
    }

    // ── RES-break-continue: break / continue / assert compilation ──

    fn vm_run(src: &str) -> crate::vm::VmError {
        let prog = parse_one(src);
        match compile(&prog) {
            Err(e) => panic!("compile error: {:?}", e),
            Ok(p) => match crate::vm::run(&p) {
                Ok(v) => panic!("expected error, got {:?}", v),
                Err(e) => e,
            },
        }
    }

    fn vm_ok(src: &str) -> Value {
        let prog = parse_one(src);
        let p = compile(&prog).expect("compiles");
        crate::vm::run(&p).expect("runs")
    }

    #[test]
    fn break_exits_while_loop() {
        // Loop would run forever without break; it exits after 3 iterations.
        let src = r#"
let i = 0;
while true {
    i = i + 1;
    if i == 3 {
        break;
    }
}
i;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn continue_skips_body_tail_in_while_loop() {
        // Accumulate only even numbers 0..10.
        let src = r#"
let i = 0;
let sum = 0;
while i < 10 {
    i = i + 1;
    if i % 2 != 0 {
        continue;
    }
    sum = sum + i;
}
sum;
"#;
        // Even numbers 2+4+6+8+10 = 30
        match vm_ok(src) {
            Value::Int(30) => {}
            other => panic!("expected Int(30), got {:?}", other),
        }
    }

    #[test]
    fn break_in_fn_while_loop() {
        let src = r#"
fn first_ge(int target) -> int {
    let i = 0;
    while true {
        i = i + 1;
        if i >= target {
            break;
        }
    }
    return i;
}
first_ge(5);
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn break_in_for_in_loop() {
        let src = r#"
let arr = [10, 20, 30, 40, 50];
let found = 0;
for x in arr {
    if x == 30 {
        found = x;
        break;
    }
}
found;
"#;
        match vm_ok(src) {
            Value::Int(30) => {}
            other => panic!("expected Int(30), got {:?}", other),
        }
    }

    #[test]
    fn continue_in_for_in_loop_skips_element() {
        let src = r#"
let arr = [1, 2, 3, 4, 5];
let sum = 0;
for x in arr {
    if x == 3 {
        continue;
    }
    sum = sum + x;
}
sum;
"#;
        // 1+2+4+5 = 12 (skipped 3)
        match vm_ok(src) {
            Value::Int(12) => {}
            other => panic!("expected Int(12), got {:?}", other),
        }
    }

    #[test]
    fn assert_passes_when_condition_true() {
        let src = r#"
let x = 5;
assert(x > 0);
x;
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn assert_fails_when_condition_false() {
        let src = r#"
let x = -1;
assert(x > 0);
x;
"#;
        let err = vm_run(src);
        assert!(
            matches!(err.kind(), crate::vm::VmError::AssertionFailed(_)),
            "expected AssertionFailed, got {:?}",
            err
        );
    }

    #[test]
    fn assert_with_custom_message() {
        let src = r#"
assert(false, "custom failure message");
"#;
        let err = vm_run(src);
        match err.kind() {
            crate::vm::VmError::AssertionFailed(msg) => {
                assert!(
                    msg.contains("custom failure message"),
                    "expected custom message in {:?}",
                    msg
                );
            }
            other => panic!("expected AssertionFailed, got {:?}", other),
        }
    }

    #[test]
    fn assert_in_fn_body_passes() {
        let src = r#"
fn check(int n) -> int {
    assert(n >= 0);
    return n * 2;
}
check(7);
"#;
        match vm_ok(src) {
            Value::Int(14) => {}
            other => panic!("expected Int(14), got {:?}", other),
        }
    }

    #[test]
    fn assert_in_fn_body_fails() {
        let src = r#"
fn check(int n) -> int {
    assert(n >= 0, "n must be non-negative");
    return n;
}
check(-1);
"#;
        let err = vm_run(src);
        assert!(
            matches!(err.kind(), crate::vm::VmError::AssertionFailed(_)),
            "expected AssertionFailed, got {:?}",
            err
        );
    }

    #[test]
    fn break_outside_loop_is_compile_error() {
        let src = "break;";
        let prog = parse_one(src);
        assert!(
            compile(&prog).is_err(),
            "expected compile error for break outside loop"
        );
    }

    #[test]
    fn continue_outside_loop_is_compile_error() {
        let src = "continue;";
        let prog = parse_one(src);
        assert!(
            compile(&prog).is_err(),
            "expected compile error for continue outside loop"
        );
    }

    #[test]
    fn nested_break_targets_inner_loop() {
        // The inner break should exit only the inner while; outer loop counts to 3.
        let src = r#"
let outer = 0;
while outer < 3 {
    let inner = 0;
    while true {
        inner = inner + 1;
        if inner == 2 {
            break;
        }
    }
    outer = outer + 1;
}
outer;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    // ---------- RES-384b / RES-291 / RES-921 / RES-152: new compile coverage ----------

    #[test]
    fn static_let_compiles_as_local() {
        let src = "static let x = 42; x;";
        match vm_ok(src) {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn const_decl_is_noop_in_vm() {
        // `const` is pre-evaluated; the bytecode should compile cleanly.
        let p = parse_one("const LIMIT = 10;");
        assert!(compile(&p).is_ok(), "const decl must compile");
    }

    #[test]
    fn live_block_body_executes() {
        // `live { ... }` compiles as a plain block in the VM.
        let src = r#"
let x = 0;
live {
    x = 5;
}
x;
"#;
        match vm_ok(src) {
            Value::Int(5) => {}
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn assume_and_invariant_are_noops() {
        // Verification-only constructs compile to no ops — program still runs.
        let src = r#"
let x = 3;
assume(x > 0, "x must be positive");
x;
"#;
        match vm_ok(src) {
            Value::Int(3) => {}
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn bytes_literal_compiles_to_bytes_value() {
        let p = parse_one(r#"b"hello";"#);
        let prog = compile(&p).expect("bytes literal must compile");
        assert!(
            prog.main
                .constants
                .iter()
                .any(|c| matches!(c, Value::Bytes(_))),
            "constant pool must contain a Bytes constant"
        );
    }

    #[test]
    fn range_expr_exclusive_produces_array() {
        let src = "let r = 0..3; r;";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(v[0], Value::Int(0)));
                assert!(matches!(v[1], Value::Int(1)));
                assert!(matches!(v[2], Value::Int(2)));
            }
            other => panic!("expected Array([0,1,2]), got {:?}", other),
        }
    }

    #[test]
    fn range_expr_inclusive_produces_array() {
        let src = "let r = 1..=3; r;";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(v[0], Value::Int(1)));
                assert!(matches!(v[2], Value::Int(3)));
            }
            other => panic!("expected Array([1,2,3]), got {:?}", other),
        }
    }

    #[test]
    fn slice_expr_basic() {
        let src = "let a = [10, 20, 30, 40]; a[1..3];";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("expected [20,30], got {:?}", other),
        }
    }

    #[test]
    fn slice_expr_inclusive() {
        let src = "let a = [10, 20, 30, 40]; a[1..=2];";
        match vm_ok(src) {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("expected [20,30], got {:?}", other),
        }
    }

    // ── DurationLiteral ──────────────────────────────────────────────────────

    #[test]
    fn duration_literal_compiles_in_live_block() {
        // DurationLiteral appears as the `deadline` of a `live within`
        // block. The bytecode compiler ignores the deadline and compiles
        // the body; the live block should run without Unsupported errors.
        match vm_ok("live within 100ms { 42; } 42;") {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    // ── NewtypeConstruct ─────────────────────────────────────────────────────

    #[test]
    fn newtype_construct_wraps_value() {
        // lower_program rewrites Meters(42) → NewtypeConstruct; the bytecode
        // compiler must not return Unsupported. Result is a Struct.
        let mut p = parse_one("newtype Meters = Int; let x = Meters(42); x;");
        crate::newtypes::lower_program(&mut p);
        let prog = compile(&p).expect("NewtypeConstruct must compile");
        let v = crate::vm::run(&prog).expect("NewtypeConstruct must run");
        assert!(
            matches!(v, Value::Struct { .. }),
            "expected Struct from newtype constructor, got {:?}",
            v
        );
    }

    // ── TryExpression (bytecode VM path) ─────────────────────────────────────

    #[test]
    fn try_unwrap_ok_result_via_vm() {
        // Build a tiny program directly in bytecode: push `Result{ok:true,
        // payload:Int(42)}`, emit TryUnwrap, emit Return. The VM must
        // leave Int(42) on the stack.
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        // Const 0 → Value::Result { ok: true, payload: Box(Int(42)) }
        main.constants.push(Value::Result {
            ok: true,
            payload: Box::new(Value::Int(42)),
        });
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(42) => {}
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn try_unwrap_some_option_via_vm() {
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        main.constants
            .push(Value::Option(Some(Box::new(Value::Int(7)))));
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(7) => {}
            other => panic!("expected Int(7), got {:?}", other),
        }
    }

    #[test]
    fn try_unwrap_err_result_early_returns() {
        // TryUnwrap on Err early-returns to the caller. When in main
        // (frames.len()==1 after pop), the VM halts with the Err value.
        use crate::bytecode::{Chunk, Op, Program};
        let mut main = Chunk::new();
        main.constants.push(Value::Result {
            ok: false,
            payload: Box::new(Value::Int(99)),
        });
        main.code.push(Op::Const(0));
        main.line_info.push(1);
        main.code.push(Op::TryUnwrap);
        main.line_info.push(1);
        // This Return is unreachable; TryUnwrap halts via early-return.
        main.code.push(Op::Return);
        main.line_info.push(1);
        let prog = Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        match crate::vm::run(&prog).expect("must run") {
            Value::Result { ok: false, payload } => {
                assert!(
                    matches!(*payload, Value::Int(99)),
                    "expected Int(99) payload, got {:?}",
                    payload
                );
            }
            other => panic!("expected Err(99), got {:?}", other),
        }
    }

    // ── NamedArg ─────────────────────────────────────────────────────────────

    #[test]
    fn named_arg_compiles_without_unsupported() {
        // NamedArg nodes appear at call sites with labelled arguments.
        // The bytecode compiler must not return Unsupported for them.
        // Compile `add(a: 3, b: 4)` — only the values matter.
        let p = parse_one("fn add(int a, int b) -> int { return a + b; } add(a: 3, b: 4);");
        let prog = compile(&p).expect("NamedArg must compile");
        match crate::vm::run(&prog).expect("must run") {
            Value::Int(7) => {}
            other => panic!("expected Int(7), got {:?}", other),
        }
    }
}
