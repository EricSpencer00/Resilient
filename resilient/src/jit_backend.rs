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
    // Step 1: locate the top-level ReturnStatement. The compiler
    // and tree walker both accept richer programs, but Phase B is
    // strictly limited.
    let return_expr = top_level_return_expr(program)?;

    // Step 2: build a `i64 () -> i64` function whose body lowers
    // `return_expr` and returns it.
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

        let result = lower_expr(return_expr, &mut bcx)?;
        bcx.ins().return_(&[result]);
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

/// Find the expression of the top-level `return EXPR;` statement.
/// Phase B requires exactly one `ReturnStatement` at top level
/// containing an `Some(expr)` payload; everything else returns
/// `Unsupported` or `EmptyProgram` so future phases can grow the
/// supported shape.
fn top_level_return_expr(program: &Node) -> Result<&Node, JitError> {
    let stmts = match program {
        Node::Program(s) => s,
        _ => return Err(JitError::Unsupported("non-Program root")),
    };
    for spanned in stmts {
        if let Node::ReturnStatement { value: Some(expr), .. } = &spanned.node {
            return Ok(expr);
        }
    }
    Err(JitError::EmptyProgram)
}

/// Lower an expression to a Cranelift `Value` of type `i64`.
fn lower_expr(node: &Node, bcx: &mut FunctionBuilder) -> Result<Value, JitError> {
    match node {
        Node::IntegerLiteral { value, .. } => Ok(bcx.ins().iconst(types::I64, *value)),
        Node::InfixExpression { left, operator, right, .. } if operator == "+" => {
            let l = lower_expr(left, bcx)?;
            let r = lower_expr(right, bcx)?;
            Ok(bcx.ins().iadd(l, r))
        }
        Node::InfixExpression { operator, .. } => {
            // The `+` arm above is the only operator Phase B supports.
            // Future tickets add `-`/`*`/`/`/`%`. Surface a clean
            // Unsupported with the operator name in the error.
            // Convert &String to a stable static via leak avoidance:
            // the error variant only carries `&'static str`, so we
            // collapse all non-+ operators to a single descriptor.
            let _ = operator; // intentionally unused — descriptor below
            Err(JitError::Unsupported("infix operator other than +"))
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

    #[test]
    fn jit_rejects_subtraction_for_now() {
        // Only `+` is wired in Phase B; other infix ops return
        // Unsupported with a clear descriptor.
        let p = parse_program("return 5 - 3;");
        let err = run(&p).unwrap_err();
        match err {
            JitError::Unsupported(msg) => assert!(msg.contains("+"), "got: {}", msg),
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
