//! RES-072 Phase A + RES-096 Phase B: Cranelift JIT backend.
//!
//! Phase A wired the dep tree, the `--jit` flag, and a stub
//! `run` that returned `JitError::Unsupported`. Phase B (this
//! revision) actually lowers a tiny subset of the AST to native
//! code and executes it:
//!
//! - `Node::IntegerLiteral { value, .. }` → `iconst`
//! - `Node::InfixExpression` with `+` → recursive lower + `iadd`
//! - `Node::ReturnStatement { value: Some(expr), .. }` → lower
//!   the expression and emit `Op::Return` for the JIT'd function
//! - Top-level `Node::Program` containing a single
//!   `ReturnStatement` is wrapped as the JIT's `main`
//!
//! Anything else returns `JitError::Unsupported(...)`. Future
//! tickets layer on let bindings (RES-097-?), control flow,
//! function calls, etc.

#![allow(dead_code)]

use std::collections::HashMap;

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use crate::Node;

/// RES-104: per-function lowering context. Threads the locals
/// map (name → cranelift Variable) and the Variable counter
/// through all lowering helpers so let bindings + identifier
/// reads compose naturally with everything from earlier phases.
struct LowerCtx {
    /// Variable index counter — Cranelift's `Variable` is a u32
    /// newtype, increment on each `let`. The counter is owned by
    /// the ctx (not global) so per-function lowering stays
    /// independent.
    next_var: u32,
    /// Currently-in-scope locals. Phase G is function-scoped:
    /// the same map is used for the whole function body. Block
    /// scoping is a future ticket.
    locals: HashMap<String, Variable>,
}

impl LowerCtx {
    fn new() -> Self {
        Self { next_var: 0, locals: HashMap::new() }
    }

    /// Reserve a fresh `Variable`, declare it on the
    /// FunctionBuilder, and remember the binding under `name`.
    /// Shadowing a previous binding just overwrites the map
    /// entry — subsequent uses get the fresh Variable.
    fn declare(&mut self, name: &str, bcx: &mut FunctionBuilder) -> Variable {
        let var = Variable::from_u32(self.next_var);
        self.next_var += 1;
        bcx.declare_var(var, types::I64);
        self.locals.insert(name.to_string(), var);
        var
    }

    fn lookup(&self, name: &str) -> Option<Variable> {
        self.locals.get(name).copied()
    }
}

/// Errors the JIT backend can surface.
#[derive(Debug, Clone, PartialEq)]
pub enum JitError {
    /// A construct outside Phase B's supported subset showed up.
    Unsupported(&'static str),
    /// `cranelift_native::builder()` failed to detect the host ISA.
    IsaInit(String),
    /// `JITModule::finalize_definitions` returned an error.
    LinkError(String),
    /// Top-level Program had no `return EXPR;` statement to JIT.
    EmptyProgram,
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JitError::Unsupported(what) => write!(f, "jit: unsupported: {}", what),
            JitError::IsaInit(msg) => write!(f, "jit: ISA init failed: {}", msg),
            JitError::LinkError(msg) => write!(f, "jit: link error: {}", msg),
            JitError::EmptyProgram => write!(f, "jit: program has no top-level return"),
        }
    }
}

impl std::error::Error for JitError {}

/// Build a fresh JITModule for the host ISA.
fn make_module() -> Result<JITModule, JitError> {
    let mut flag_builder = settings::builder();
    // Default cranelift settings work for our needs; setting these
    // two explicitly avoids surprises on platforms where the
    // defaults change between cranelift versions.
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    flag_builder
        .set("is_pic", "false")
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    let isa_builder = cranelift_native::builder()
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| JitError::IsaInit(e.to_string()))?;
    let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    Ok(JITModule::new(builder))
}

/// RES-072 + RES-096: compile a Resilient `Program` to native
/// code and execute it. Today's subset:
/// `return <int-arith expression>;` at the top level, where the
/// expression uses only integer literals and `+`.
pub fn run(program: &Node) -> Result<i64, JitError> {
    // Step 1: locate the top-level statement slice we need to
    // lower. The compiler and tree walker both accept richer
    // programs; the JIT path is still a strict subset.
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(JitError::Unsupported("non-Program root")),
    };

    // Step 2: build a `i64 () -> i64` function whose body lowers
    // the program statements (possibly including IfStatement) and
    // emits the appropriate return_ per arm.
    let mut module = make_module()?;
    let mut sig = module.make_signature();
    sig.returns.push(AbiParam::new(types::I64));
    let func_id = module
        .declare_function("main", Linkage::Local, &sig)
        .map_err(|e| JitError::LinkError(e.to_string()))?;

    let mut ctx = module.make_context();
    ctx.func.signature = sig;
    let mut builder_ctx = FunctionBuilderContext::new();
    {
        let mut bcx = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
        let entry = bcx.create_block();
        bcx.append_block_params_for_function_params(entry);
        bcx.switch_to_block(entry);
        bcx.seal_block(entry);

        // RES-102 + RES-104: walk top-level statements and emit
        // returns inline. compile_statements handles IfStatement
        // (each arm gets its own block + return_), let bindings
        // (declare a Variable + def_var), and the trailing
        // ReturnStatement.
        let mut ctx = LowerCtx::new();
        compile_statements(stmts, &mut bcx, &mut ctx)?;
        bcx.finalize();
    }

    module
        .define_function(func_id, &mut ctx)
        .map_err(|e| JitError::LinkError(e.to_string()))?;
    module.clear_context(&mut ctx);
    module
        .finalize_definitions()
        .map_err(|e| JitError::LinkError(e.to_string()))?;

    // Step 3: get the function pointer and call it.
    let raw = module.get_finalized_function(func_id);
    // SAFETY: `raw` points at a freshly-finalized function with
    // signature `extern "C" fn() -> i64`; we constructed that
    // signature ourselves above. The JITModule keeps the code
    // alive — `module` outlives this call.
    let f: unsafe extern "C" fn() -> i64 = unsafe { std::mem::transmute(raw) };
    let result = unsafe { f() };
    Ok(result)
}

/// RES-102 + RES-103: walk a slice of top-level statements and
/// emit Cranelift instructions including the function's `return_`.
///
/// Supported shapes (grows ticket by ticket):
/// 1. A single `ReturnStatement { value: Some(expr) }`
///    → lowers the expression and emits `return_`.
/// 2. An `IfStatement`. Phase F (RES-103) handles four sub-cases:
///    both arms terminate, then-only terminates,
///    else-only terminates, neither terminates. For any
///    fallthrough case the surrounding compile_node_list keeps
///    walking from the merge block. If the walk completes
///    without ever emitting a return, compile_statements raises
///    `EmptyProgram` ("program has no top-level return") — same
///    behavior as a program with no return statement at all.
fn compile_statements(
    stmts: &[crate::Spanned<Node>],
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Result<(), JitError> {
    // Top-level statements are Spanned<Node>; Block bodies are
    // raw Node. Strip the wrapper here and delegate to the shared
    // walker so the lowering logic isn't duplicated.
    let nodes: Vec<&Node> = stmts.iter().map(|s| &s.node).collect();
    let returned = compile_node_list(&nodes, bcx, ctx)?;
    if !returned {
        return Err(JitError::EmptyProgram);
    }
    Ok(())
}

/// Walks a slice of statement nodes and emits cranelift
/// instructions. Returns `Ok(true)` when the walk emitted a
/// terminator (a `return_`, or an if/else where both arms
/// terminated). Returns `Ok(false)` when the walk completed
/// without emitting any terminator — the caller decides whether
/// that's an error (top-level) or a fallthrough (inside a Block).
fn compile_node_list(
    stmts: &[&Node],
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Result<bool, JitError> {
    for node in stmts {
        match node {
            Node::ReturnStatement { value: Some(expr), .. } => {
                let v = lower_expr(expr, bcx, ctx)?;
                bcx.ins().return_(&[v]);
                return Ok(true);
            }
            Node::IfStatement { condition, consequence, alternative, .. } => {
                let if_terminated = lower_if_statement(
                    condition,
                    consequence,
                    alternative.as_deref(),
                    bcx,
                    ctx,
                )?;
                if if_terminated {
                    // Both arms returned — function exits, no
                    // statements after the if can run.
                    return Ok(true);
                }
                // RES-103: at least one arm fell through; the
                // builder is now positioned at the merge block.
                // Keep walking — trailing statements lower into
                // the merge block.
                continue;
            }
            // RES-104: `let NAME = EXPR;` — lower the RHS, declare
            // a fresh Variable, and bind NAME to it. Subsequent
            // identifier reads via lower_expr will use_var the
            // same Variable.
            Node::LetStatement { name, value, .. } => {
                let v = lower_expr(value, bcx, ctx)?;
                let var = ctx.declare(name, bcx);
                bcx.def_var(var, v);
                continue;
            }
            // Skip statements with no JIT-relevant effect for now;
            // a future phase will lower expression statements,
            // reassignment, while loops, etc.
            _ => continue,
        }
    }
    Ok(false)
}

/// RES-102 + RES-103: lower an IfStatement.
///
/// Returns `Ok(true)` when both arms emit terminators (function
/// exits from each arm — no merge block needed). Returns
/// `Ok(false)` when at least one arm falls through; in that case
/// the function builder is positioned at the merge block on
/// return so the caller can continue lowering trailing
/// statements there.
///
/// Cranelift block dance:
///   brif(cond, then_block, &[], else_block, &[])
///   then_block: lower then-arm; emits return_ OR jump merge
///   else_block: lower else-arm (or missing → straight to merge);
///               emits return_ OR jump merge
///   merge_block (if either arm fell through): switch + seal
///
/// No phi nodes are needed because lower_if_statement doesn't
/// produce an SSA value yet — that's a future "if as expression"
/// phase.
fn lower_if_statement(
    condition: &Node,
    consequence: &Node,
    alternative: Option<&Node>,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Result<bool, JitError> {
    let cond_val = lower_expr(condition, bcx, ctx)?;

    let then_block = bcx.create_block();
    let else_block = bcx.create_block();
    // Create merge_block up-front so each arm can jump to it
    // inline. If neither arm needs the merge (both terminate)
    // we'll just never switch to it — Cranelift doesn't require
    // unused blocks to be sealed before finalize.
    let merge_block = bcx.create_block();

    bcx.ins().brif(cond_val, then_block, &[], else_block, &[]);

    // then-arm
    bcx.switch_to_block(then_block);
    bcx.seal_block(then_block);
    let then_terminated = lower_block_or_stmt(consequence, bcx, ctx)?;
    if !then_terminated {
        // then-arm fell through — jump to merge so the trailing
        // statements after the if can run.
        bcx.ins().jump(merge_block, &[]);
    }

    // else-arm
    bcx.switch_to_block(else_block);
    bcx.seal_block(else_block);
    let else_terminated = match alternative {
        Some(alt) => lower_block_or_stmt(alt, bcx, ctx)?,
        // Bare `if` with no else: the else-block has nothing to
        // lower and falls through immediately. RES-103 treats
        // this as a fallthrough (Phase E used to reject it).
        None => false,
    };
    if !else_terminated {
        bcx.ins().jump(merge_block, &[]);
    }

    if then_terminated && else_terminated {
        // Both arms exited — merge_block has no predecessors, so
        // we'll never use it. Cranelift accepts unused blocks at
        // finalize time as long as they're sealed; seal it here
        // to keep things tidy. (FunctionBuilder will skip
        // emitting code for it.)
        bcx.seal_block(merge_block);
        return Ok(true);
    }

    // At least one arm fell through. Switch to merge so the
    // caller's compile_node_list lowers trailing statements
    // here, and seal — both predecessor jumps were emitted
    // above (or one arm terminated and the merge has a single
    // predecessor jump).
    bcx.switch_to_block(merge_block);
    bcx.seal_block(merge_block);
    Ok(false)
}

/// Lower a Block, or a single statement (in case `else if` chains
/// ever land — for now `consequence` is always a Block from the
/// parser). Recurses into compile_statements so the same set of
/// statement shapes is supported uniformly.
/// Lower a Block (typical) or single statement (for `else if`,
/// where the parser gives a nested IfStatement directly as
/// `alternative`). Returns Ok(true) when a terminator (return)
/// was emitted, Ok(false) when the block fell through.
fn lower_block_or_stmt(
    node: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Result<bool, JitError> {
    match node {
        Node::Block { stmts, .. } => {
            let refs: Vec<&Node> = stmts.iter().collect();
            compile_node_list(&refs, bcx, ctx)
        }
        Node::IfStatement { condition, consequence, alternative, .. } => {
            // RES-103: an if "terminates" only if both arms did.
            // Otherwise the merge block is now active and the
            // surrounding block's caller may want to keep walking.
            lower_if_statement(condition, consequence, alternative.as_deref(), bcx, ctx)
        }
        Node::ReturnStatement { value: Some(expr), .. } => {
            let v = lower_expr(expr, bcx, ctx)?;
            bcx.ins().return_(&[v]);
            Ok(true)
        }
        _ => Err(JitError::Unsupported(node_kind(node))),
    }
}

/// Lower an expression to a Cranelift `Value` of type `i64`.
fn lower_expr(
    node: &Node,
    bcx: &mut FunctionBuilder,
    ctx: &mut LowerCtx,
) -> Result<Value, JitError> {
    match node {
        Node::IntegerLiteral { value, .. } => Ok(bcx.ins().iconst(types::I64, *value)),
        // RES-100: bool literals lower to i64 0/1 — matches how
        // the bytecode VM materializes booleans, so the JIT result
        // is identical when the program runs on either backend.
        Node::BooleanLiteral { value, .. } => {
            Ok(bcx.ins().iconst(types::I64, if *value { 1 } else { 0 }))
        }
        // RES-104: identifier read — look up the Variable in the
        // locals map and use_var. Cranelift's SSA construction
        // routes the right value to this use.
        Node::Identifier { name, .. } => match ctx.lookup(name) {
            Some(var) => Ok(bcx.use_var(var)),
            None => Err(JitError::Unsupported("identifier not in scope")),
        },
        // RES-099: lower all four signed integer infix ops + RES-100:
        // the six comparison ops. Same recursive shape — recurse on
        // left + right, then emit the matching Cranelift instruction.
        // Note: `sdiv`/`srem` exhibit UB at the IR level when rhs == 0;
        // a future ticket should emit a runtime check that traps or
        // returns a sentinel. For now this matches what the bytecode
        // VM does WITHOUT line attribution on the JIT path.
        Node::InfixExpression { left, operator, right, .. } => {
            let op_str = operator.as_str();
            // Validate first so we can short-circuit Unsupported
            // before recursing into the operands.
            if !matches!(
                op_str,
                "+" | "-" | "*" | "/" | "%" | "==" | "!=" | "<" | "<=" | ">" | ">="
            ) {
                return Err(JitError::Unsupported(
                    "infix operator other than +,-,*,/,%,==,!=,<,<=,>,>=",
                ));
            }
            let l = lower_expr(left, bcx, ctx)?;
            let r = lower_expr(right, bcx, ctx)?;
            Ok(match op_str {
                "+" => bcx.ins().iadd(l, r),
                "-" => bcx.ins().isub(l, r),
                "*" => bcx.ins().imul(l, r),
                "/" => bcx.ins().sdiv(l, r),
                "%" => bcx.ins().srem(l, r),
                // RES-100: comparisons return Cranelift's i8 0/1.
                // uextend widens to i64 so the function signature
                // (returns i64) stays uniform regardless of which
                // op the user wrote.
                cmp => {
                    let cc = match cmp {
                        "==" => IntCC::Equal,
                        "!=" => IntCC::NotEqual,
                        "<" => IntCC::SignedLessThan,
                        "<=" => IntCC::SignedLessThanOrEqual,
                        ">" => IntCC::SignedGreaterThan,
                        ">=" => IntCC::SignedGreaterThanOrEqual,
                        _ => unreachable!("validated above"),
                    };
                    let raw = bcx.ins().icmp(cc, l, r);
                    bcx.ins().uextend(types::I64, raw)
                }
            })
        }
        _ => Err(JitError::Unsupported(node_kind(node))),
    }
}

fn node_kind(n: &Node) -> &'static str {
    match n {
        Node::Program(_) => "Program",
        Node::Function { .. } => "Function",
        Node::LetStatement { .. } => "LetStatement",
        Node::ReturnStatement { .. } => "ReturnStatement",
        Node::IfStatement { .. } => "IfStatement",
        Node::WhileStatement { .. } => "WhileStatement",
        Node::Identifier { .. } => "Identifier",
        Node::IntegerLiteral { .. } => "IntegerLiteral",
        Node::FloatLiteral { .. } => "FloatLiteral",
        Node::StringLiteral { .. } => "StringLiteral",
        Node::BooleanLiteral { .. } => "BooleanLiteral",
        Node::PrefixExpression { .. } => "PrefixExpression",
        Node::InfixExpression { .. } => "InfixExpression",
        Node::CallExpression { .. } => "CallExpression",
        Node::Block { .. } => "Block",
        Node::ExpressionStatement { .. } => "ExpressionStatement",
        _ => "<other>",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_program(src: &str) -> Node {
        let (program, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        program
    }

    #[test]
    fn jit_returns_constant_42() {
        let p = parse_program("return 42;");
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn jit_adds_two_constants() {
        let p = parse_program("return 2 + 3;");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn jit_adds_three_constants() {
        // Confirms the recursive lowering composes left-associatively.
        let p = parse_program("return 1 + 2 + 4;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    // RES-104 closed Phase G — let bindings + identifier reads
    // both work now. The test that was here pinning the
    // unsupported case was retired; the equivalent positive
    // test (jit_let_and_use, below) replaces it.

    #[test]
    fn jit_undeclared_identifier_unsupported() {
        // An identifier read with no matching `let` is still
        // unsupported in Phase G — a future ticket can promote
        // this to a richer "scope error" diagnostic, but for
        // now Unsupported with the descriptor is enough.
        let p = parse_program("return undefined_var;");
        match run(&p).unwrap_err() {
            JitError::Unsupported(msg) => assert!(
                msg.contains("identifier not in scope"),
                "expected scope descriptor, got: {}",
                msg
            ),
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    // RES-100 closed Phase D — comparison ops work now too.
    // What's still unsupported at the expression level: prefix
    // ops (`-x`, `!x`), identifiers, calls, blocks. This test
    // pins one of those (prefix `-`) so the descriptor list keeps
    // being a useful diagnostic for users.
    #[test]
    fn jit_rejects_prefix_for_now() {
        let p = parse_program("return -5;");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(
                msg.contains("Prefix"),
                "expected node-kind in descriptor, got: {}",
                msg
            ),
            _ => panic!("expected Unsupported, got {:?}", err),
        }
    }

    // ---------- RES-099: Sub/Mul/Div/Mod ----------

    #[test]
    fn jit_subtraction() {
        let p = parse_program("return 10 - 3;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_multiplication() {
        let p = parse_program("return 6 * 7;");
        assert_eq!(run(&p).unwrap(), 42);
    }

    #[test]
    fn jit_division() {
        let p = parse_program("return 100 / 4;");
        assert_eq!(run(&p).unwrap(), 25);
    }

    #[test]
    fn jit_modulo() {
        let p = parse_program("return 17 % 5;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_arith_chain_respects_precedence() {
        // Pratt precedence: `*` binds tighter than `+`, so this
        // parses as `2 + (3 * 4)` = 14. Exercises composition of
        // two different ops without needing explicit grouping.
        let p = parse_program("return 2 + 3 * 4;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    #[test]
    fn jit_arith_chain_all_four_ops() {
        // 20 / 4 = 5; 5 * 3 = 15; 15 - 2 = 13; 13 + 1 = 14.
        // Verifies sdiv/imul/isub/iadd compose left-to-right
        // within their precedence tier.
        let p = parse_program("return 20 / 4 * 3 - 2 + 1;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    // ---------- RES-100: comparisons + bool literals ----------

    #[test]
    fn jit_lt_returns_zero_for_false() {
        // 5 < 3 is false → Cranelift's icmp returns 0, uextend
        // widens to i64(0).
        let p = parse_program("return 5 < 3;");
        assert_eq!(run(&p).unwrap(), 0);
    }

    #[test]
    fn jit_lt_returns_one_for_true() {
        let p = parse_program("return 3 < 5;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_eq_int() {
        let true_case = parse_program("return 7 == 7;");
        assert_eq!(run(&true_case).unwrap(), 1);
        let false_case = parse_program("return 7 == 8;");
        assert_eq!(run(&false_case).unwrap(), 0);
    }

    #[test]
    fn jit_ne_int() {
        let true_case = parse_program("return 1 != 2;");
        assert_eq!(run(&true_case).unwrap(), 1);
        let false_case = parse_program("return 1 != 1;");
        assert_eq!(run(&false_case).unwrap(), 0);
    }

    #[test]
    fn jit_le_ge_boundary_equality() {
        // <= and >= must each return 1 at boundary equality and
        // 0 just past the boundary.
        let le = parse_program("return 5 <= 5;");
        assert_eq!(run(&le).unwrap(), 1);
        let ge = parse_program("return 5 >= 5;");
        assert_eq!(run(&ge).unwrap(), 1);
        let le_strict = parse_program("return 6 <= 5;");
        assert_eq!(run(&le_strict).unwrap(), 0);
        let ge_strict = parse_program("return 4 >= 5;");
        assert_eq!(run(&ge_strict).unwrap(), 0);
    }

    #[test]
    fn jit_bool_literal_true() {
        let p = parse_program("return true;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_bool_literal_false() {
        let p = parse_program("return false;");
        assert_eq!(run(&p).unwrap(), 0);
    }

    #[test]
    fn jit_compare_with_arith() {
        // Composes the RES-099 arith lowerings with the new
        // comparison lowering. Pratt: `+` binds tighter than `<`,
        // so this is `(2 + 3) < 10` = true → 1.
        let p = parse_program("return 2 + 3 < 10;");
        assert_eq!(run(&p).unwrap(), 1);
    }

    // ---------- RES-102: if/else with brif ----------

    #[test]
    fn jit_if_then_returns() {
        // `if (1 < 2) { return 7; } return 9;` — Phase E requires
        // both arms to return, so phrase as if-else (this test
        // documents the natural form users reach for).
        let p = parse_program("if (1 < 2) { return 7; } else { return 9; }");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_if_else_returns() {
        let p = parse_program("if (1 > 2) { return 7; } else { return 9; }");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_if_with_arith_cond() {
        // The condition exercises both arith (5+5) and comparison
        // (==) lowerings before reaching the if. true → 1 arm.
        let p = parse_program("if (5 + 5 == 10) { return 1; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 1);
    }

    #[test]
    fn jit_if_with_bool_literal_cond() {
        // BooleanLiteral lowers to iconst 0/1, which is exactly
        // what brif consumes. No icmp required.
        let p = parse_program("if (true) { return 42; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 42);
        let p2 = parse_program("if (false) { return 42; } else { return 0; }");
        assert_eq!(run(&p2).unwrap(), 0);
    }

    // ---------- RES-104: let bindings + identifier reads ----------

    #[test]
    fn jit_let_and_use() {
        // Smallest meaningful test: bind a value, then use it.
        let p = parse_program("let x = 5; return x + 10;");
        assert_eq!(run(&p).unwrap(), 15);
    }

    #[test]
    fn jit_let_in_arith() {
        // Two locals in an arithmetic expression. Pratt: `*`
        // binds tighter than `+`, so this is `a * b + 2` →
        // (3 * 4) + 2 = 14.
        let p = parse_program("let a = 3; let b = 4; return a * b + 2;");
        assert_eq!(run(&p).unwrap(), 14);
    }

    #[test]
    fn jit_let_in_if_condition() {
        // Identifier read inside an if condition: composes
        // RES-100 comparison + RES-104 lookup.
        let p = parse_program("let x = 5; if (x > 0) { return x; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 5);
    }

    #[test]
    fn jit_let_inside_arm() {
        // `let` inside a then-arm — the LowerCtx threads down
        // through lower_block_or_stmt, so the local is visible
        // for the arm-local return. Phase G is function-scoped,
        // so the binding outlives the arm but no test exercises
        // that yet (would need post-if usage).
        let p = parse_program("if (1 < 2) { let y = 7; return y; } else { return 0; }");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_let_shadowing() {
        // `let x = 1; let x = 2; return x;` — second `let x`
        // overwrites the HashMap entry, so the use_var picks
        // up the fresh Variable. Function-scoped semantics
        // mean shadowing is just rebinding.
        let p = parse_program("let x = 1; let x = 2; return x;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_let_used_after_if_fallthrough() {
        // Combines RES-103 fallthrough with RES-104 locals:
        // bind x, conditionally early-return, otherwise use x
        // in the trailing return. Proves the LowerCtx survives
        // across the merge_block.
        let p = parse_program("let x = 7; if (false) { return 0; } return x + 1;");
        assert_eq!(run(&p).unwrap(), 8);
    }

    // ---------- RES-103: merge block + fallthrough ----------

    #[test]
    fn jit_if_then_returns_else_falls_through() {
        // then-arm taken, returns 7. The else-arm falls through
        // to the merge block, where the trailing `return 9;`
        // lowers. Tests the "then terminates, else doesn't" path.
        let p = parse_program("if (1 < 2) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_then_falls_through_else_returns() {
        // Inverse of above: condition false → else taken (returns
        // 9). When then-arm executes a no-return body, the
        // fallthrough hits the trailing return. We can't easily
        // construct "then has no return" without let bindings
        // (RES-104) — use bare-if instead, which is also a
        // fallthrough-from-then case.
        let p = parse_program("if (1 > 2) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_bare_if_with_fallthrough_false_branch() {
        // No else; condition false → fallthrough to trailing
        // return. This is the case Phase E rejected with
        // "bare `if` without else"; Phase F accepts it.
        let p = parse_program("if (false) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 9);
    }

    #[test]
    fn jit_bare_if_with_fallthrough_true_branch() {
        // No else; condition true → then-arm returns 7. The
        // trailing return is unreachable but still lowers
        // (cranelift is happy with dead blocks).
        let p = parse_program("if (true) { return 7; } return 9;");
        assert_eq!(run(&p).unwrap(), 7);
    }

    #[test]
    fn jit_two_ifs_in_sequence() {
        // First if falls through (false branch), second if
        // returns. Proves the merge_block correctly hands
        // control back to compile_node_list which then walks
        // the second if. A nice end-to-end test of the
        // fallthrough mechanic.
        let p = parse_program("if (false) { return 1; } if (true) { return 2; } return 3;");
        assert_eq!(run(&p).unwrap(), 2);
    }

    #[test]
    fn jit_nested_if_in_then_arm() {
        // A nested if inside the then-arm. The inner if must also
        // have both branches returning per Phase E. This proves
        // the recursive lower_block_or_stmt handles nested control
        // flow without specialcasing.
        let p = parse_program(
            "if (1 < 2) { if (3 < 4) { return 1; } else { return 2; } } else { return 9; }",
        );
        assert_eq!(run(&p).unwrap(), 1);
    }

    // RES-103 lifted Phase E's "both arms must return" rule.
    // The two old tests (jit_rejects_if_without_else,
    // jit_rejects_if_arm_without_return) pinned shapes that
    // Phase F now accepts via the merge_block. Below: the
    // shape that's STILL rejected — an if that doesn't return
    // AND has nothing after it. The function never returns.

    #[test]
    fn jit_if_with_no_return_anywhere_is_empty_program() {
        // `if (false) { let x = 1; }` — no return in either
        // arm, no trailing statement. Function never returns,
        // so this surfaces as EmptyProgram (same error a bare
        // `let x = 1;` would).
        let p = parse_program("if (1 < 2) { let x = 1; }");
        assert_eq!(run(&p).unwrap_err(), JitError::EmptyProgram);
    }

    #[test]
    fn jit_empty_program_is_clean_error() {
        let p = parse_program("let x = 1;");
        let err = run(&p).unwrap_err();
        assert_eq!(err, JitError::EmptyProgram);
    }

    #[test]
    fn jit_error_display_is_descriptive() {
        assert_eq!(
            JitError::Unsupported("test").to_string(),
            "jit: unsupported: test"
        );
        assert_eq!(JitError::EmptyProgram.to_string(), "jit: program has no top-level return");
        assert_eq!(
            JitError::IsaInit("foo".into()).to_string(),
            "jit: ISA init failed: foo"
        );
    }
}
