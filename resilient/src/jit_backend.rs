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

use cranelift::prelude::*;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use crate::Node;

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

        // RES-102: walk top-level statements and emit returns
        // inline. compile_statements handles IfStatement (each arm
        // gets its own block + return_) and the trailing
        // ReturnStatement.
        compile_statements(stmts, &mut bcx)?;
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

/// RES-102: walk a slice of top-level statements and emit
/// Cranelift instructions including the function's `return_`.
///
/// Phase E supports two top-level shapes:
/// 1. A single `ReturnStatement { value: Some(expr) }`
///    → lowers the expression and emits `return_`.
/// 2. An `IfStatement` whose then-arm AND else-arm each contain
///    a `ReturnStatement` → lowers via a brif into two blocks,
///    each block ends with `return_`. A trailing return after
///    the if is allowed (acts as a fallthrough), but currently
///    Phase E requires both arms of the if to return so the
///    trailing path is unreachable. (Future phase can lift this.)
///
/// Anything else is `Unsupported(...)` or `EmptyProgram` so the
/// supported shape grows ticket by ticket.
fn compile_statements(
    stmts: &[crate::Spanned<Node>],
    bcx: &mut FunctionBuilder,
) -> Result<(), JitError> {
    // Top-level statements are Spanned<Node>; Block bodies are
    // raw Node. Strip the wrapper here and delegate to the shared
    // walker so the lowering logic isn't duplicated.
    let nodes: Vec<&Node> = stmts.iter().map(|s| &s.node).collect();
    let returned = compile_node_list(&nodes, bcx)?;
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
) -> Result<bool, JitError> {
    for node in stmts {
        match node {
            Node::ReturnStatement { value: Some(expr), .. } => {
                let v = lower_expr(expr, bcx)?;
                bcx.ins().return_(&[v]);
                return Ok(true);
            }
            Node::IfStatement { condition, consequence, alternative, .. } => {
                lower_if_statement(condition, consequence, alternative.as_deref(), bcx)?;
                // lower_if_statement enforces that both arms ended
                // in a return, so the current block is filled and
                // the if itself counts as a terminator for the
                // surrounding statement list.
                return Ok(true);
            }
            // Skip statements with no JIT-relevant effect for now;
            // a future phase will lower let-bindings, expression
            // statements, etc.
            _ => continue,
        }
    }
    Ok(false)
}

/// RES-102: lower an IfStatement. Both arms must end in a return
/// in Phase E — see compile_statements docs.
///
/// Cranelift block dance:
///   brif(cond, then_block, &[], else_block, &[])
///   then_block: lower then-arm; emits return_
///   else_block: lower else-arm; emits return_
///
/// With no block params and no back-edges, both blocks can be
/// sealed immediately after creation/branch.
fn lower_if_statement(
    condition: &Node,
    consequence: &Node,
    alternative: Option<&Node>,
    bcx: &mut FunctionBuilder,
) -> Result<(), JitError> {
    let cond_val = lower_expr(condition, bcx)?;

    let then_block = bcx.create_block();
    let else_block = bcx.create_block();

    bcx.ins().brif(cond_val, then_block, &[], else_block, &[]);

    // then-arm
    bcx.switch_to_block(then_block);
    bcx.seal_block(then_block);
    let then_terminated = lower_block_or_stmt(consequence, bcx)?;
    if !then_terminated {
        // Phase E requires both arms to return so the merge block
        // doesn't need phi nodes. (Future phase will lift this.)
        return Err(JitError::Unsupported(
            "if/else arm without a return — Phase E requires both arms to return",
        ));
    }

    // else-arm
    bcx.switch_to_block(else_block);
    bcx.seal_block(else_block);
    match alternative {
        Some(alt) => {
            let else_terminated = lower_block_or_stmt(alt, bcx)?;
            if !else_terminated {
                return Err(JitError::Unsupported(
                    "if/else arm without a return — Phase E requires both arms to return",
                ));
            }
        }
        None => {
            return Err(JitError::Unsupported(
                "bare `if` without `else` — Phase E requires both arms to return",
            ));
        }
    }

    Ok(())
}

/// Lower a Block, or a single statement (in case `else if` chains
/// ever land — for now `consequence` is always a Block from the
/// parser). Recurses into compile_statements so the same set of
/// statement shapes is supported uniformly.
/// Lower a Block (typical) or single statement (for `else if`,
/// where the parser gives a nested IfStatement directly as
/// `alternative`). Returns Ok(true) when a terminator (return)
/// was emitted, Ok(false) when the block fell through.
fn lower_block_or_stmt(node: &Node, bcx: &mut FunctionBuilder) -> Result<bool, JitError> {
    match node {
        Node::Block { stmts, .. } => {
            let refs: Vec<&Node> = stmts.iter().collect();
            compile_node_list(&refs, bcx)
        }
        Node::IfStatement { condition, consequence, alternative, .. } => {
            lower_if_statement(condition, consequence, alternative.as_deref(), bcx)?;
            Ok(true)
        }
        Node::ReturnStatement { value: Some(expr), .. } => {
            let v = lower_expr(expr, bcx)?;
            bcx.ins().return_(&[v]);
            Ok(true)
        }
        _ => Err(JitError::Unsupported(node_kind(node))),
    }
}

/// Lower an expression to a Cranelift `Value` of type `i64`.
fn lower_expr(node: &Node, bcx: &mut FunctionBuilder) -> Result<Value, JitError> {
    match node {
        Node::IntegerLiteral { value, .. } => Ok(bcx.ins().iconst(types::I64, *value)),
        // RES-100: bool literals lower to i64 0/1 — matches how
        // the bytecode VM materializes booleans, so the JIT result
        // is identical when the program runs on either backend.
        Node::BooleanLiteral { value, .. } => {
            Ok(bcx.ins().iconst(types::I64, if *value { 1 } else { 0 }))
        }
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
            let l = lower_expr(left, bcx)?;
            let r = lower_expr(right, bcx)?;
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

    #[test]
    fn jit_rejects_let_for_now() {
        let p = parse_program("let x = 1; return x;");
        // The walk hits ReturnStatement first via top_level_return_expr;
        // its expr is an Identifier which is unsupported in Phase B.
        let err = run(&p).unwrap_err();
        assert!(
            matches!(err, JitError::Unsupported(_)),
            "expected Unsupported, got {:?}",
            err
        );
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

    #[test]
    fn jit_rejects_if_without_else() {
        // Phase E doesn't yet support fallthrough — bare `if`
        // (no else) returns Unsupported with a clear descriptor.
        let p = parse_program("if (1 < 2) { return 7; }");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(
                msg.contains("bare `if`") || msg.contains("without `else`"),
                "expected Phase E descriptor, got: {}",
                msg
            ),
            _ => panic!("expected Unsupported, got {:?}", err),
        }
    }

    #[test]
    fn jit_rejects_if_arm_without_return() {
        // `if (cond) { /* no return */ } else { return X; }` is
        // also Unsupported in Phase E — both arms must return.
        let p = parse_program("if (1 < 2) { let x = 1; } else { return 9; }");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(
                msg.contains("without a return"),
                "expected Phase E without-return descriptor, got: {}",
                msg
            ),
            _ => panic!("expected Unsupported, got {:?}", err),
        }
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
