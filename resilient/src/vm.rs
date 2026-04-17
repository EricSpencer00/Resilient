//! RES-076 + RES-081: stack-based bytecode VM.
//!
//! Walks a `Program` produced by `compiler::compile`. The execution
//! model is dead simple: an operand stack of `Value`s, a single
//! locals slab, and a stack of `CallFrame`s. Each frame records the
//! chunk it's executing, its pc, and the base index into the locals
//! slab (so LoadLocal/StoreLocal indices are frame-relative).
//!
//! There are no forward/backward jumps yet (control-flow ops land
//! in RES-083), but RES-081 adds `Call` / `ReturnFromCall` so every
//! non-trivial program that doesn't branch can now run under `--vm`.

#![allow(dead_code)]

use crate::bytecode::{Chunk, Op, Program};
use crate::Value;

/// Errors the VM can surface at runtime. Like `CompileError`, the
/// `&'static str` payloads describe the offending op without
/// allocating per-error.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    EmptyStack,
    DivideByZero,
    TypeMismatch(&'static str),
    LocalOutOfBounds(u16),
    ConstantOutOfBounds(u16),
    /// RES-081: `Op::Call(idx)` with `idx` outside `program.functions`.
    FunctionOutOfBounds(u16),
    /// RES-081: `ReturnFromCall` with no caller — either the program
    /// emitted it at the top level, or a fn-body underflow.
    CallStackUnderflow,
    /// RES-081: call stack depth exceeded a safety cap (defense
    /// against runaway recursion — infinite fib etc.).
    CallStackOverflow,
    /// RES-083: a jump's target PC fell outside the current chunk.
    JumpOutOfBounds,
    /// RES-091: wraps any other variant with the source line of the
    /// instruction that produced it. Lets the user see
    /// `vm: divide by zero (line 5)` instead of unattributed errors.
    AtLine {
        line: u32,
        kind: Box<VmError>,
    },
}

impl VmError {
    /// RES-091: strip any `AtLine` wrappers and return the underlying
    /// error variant. Tests that match on the *kind* of error
    /// (without caring about location) call this first.
    pub fn kind(&self) -> &VmError {
        match self {
            VmError::AtLine { kind, .. } => kind.kind(),
            other => other,
        }
    }
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::EmptyStack => write!(f, "vm: operand stack underflow"),
            VmError::DivideByZero => write!(f, "vm: divide by zero"),
            VmError::TypeMismatch(what) => write!(f, "vm: type mismatch in {}", what),
            VmError::LocalOutOfBounds(i) => write!(f, "vm: local {} out of bounds", i),
            VmError::ConstantOutOfBounds(i) => write!(f, "vm: constant {} out of bounds", i),
            VmError::FunctionOutOfBounds(i) => write!(f, "vm: function {} out of bounds", i),
            VmError::CallStackUnderflow => write!(f, "vm: call stack underflow"),
            VmError::CallStackOverflow => write!(f, "vm: call stack overflow (>1024 frames)"),
            VmError::JumpOutOfBounds => write!(f, "vm: jump target out of bounds"),
            VmError::AtLine { line, kind } => write!(f, "{} (line {})", kind, line),
        }
    }
}

impl std::error::Error for VmError {}

/// RES-091: wrap a runtime error with the source line of the
/// instruction at `pc`. If `line_info` is shorter than `pc` (which
/// shouldn't happen for well-formed chunks but defensive code is
/// cheap), or the recorded line is 0 (sentinel for "synthetic"),
/// pass the error through unchanged.
fn err_at(line_info: &[u32], pc: usize, e: VmError) -> VmError {
    // pc was already incremented past the failing op when the error
    // fired, so look back one step for the offending instruction's
    // line.
    let op_pc = pc.saturating_sub(1);
    match line_info.get(op_pc) {
        Some(&line) if line > 0 => VmError::AtLine { line, kind: Box::new(e) },
        _ => e,
    }
}

/// Cap on concurrent call frames. Prevents unbounded native-stack
/// growth on pathologically-recursive input (test case for
/// `VmError::CallStackOverflow`).
const MAX_CALL_DEPTH: usize = 1024;

/// RES-081: one active function invocation. `chunk_idx = usize::MAX`
/// marks the `main` frame; any other value indexes into
/// `program.functions`.
#[derive(Debug)]
struct CallFrame {
    /// Index into `program.functions`, or `usize::MAX` for the main
    /// chunk. Kept as an index (not a `*const Chunk`) to stay safe
    /// across Vec growth.
    chunk_idx: usize,
    /// Program counter within this frame's chunk.
    pc: usize,
    /// Base offset into the shared `locals` slab. LoadLocal(idx)
    /// resolves to `locals[locals_base + idx]`.
    locals_base: usize,
}

/// Run a compiled program. Returns the value left on the operand
/// stack when the outer `Op::Return` fires (`Value::Void` if empty).
///
/// RES-091: errors are wrapped with `VmError::AtLine` carrying the
/// source line of the failing instruction (looked up via
/// `chunk.line_info`). The wrapping happens once at the outer return,
/// using the `(chunk_idx, pc)` snapshot taken at the top of each
/// dispatch iteration — keeps every inner `?` and `return Err(...)`
/// site untouched.
pub fn run(program: &Program) -> Result<Value, VmError> {
    // Sentinel for "no failure attributable yet" — main chunk @ pc 0.
    let mut last_pc: (usize, usize) = (usize::MAX, 0);
    match run_inner(program, &mut last_pc) {
        Ok(v) => Ok(v),
        Err(e) => {
            let line_info: &[u32] = if last_pc.0 == usize::MAX {
                &program.main.line_info
            } else {
                program
                    .functions
                    .get(last_pc.0)
                    .map(|f| f.chunk.line_info.as_slice())
                    .unwrap_or(&[])
            };
            Err(err_at(line_info, last_pc.1, e))
        }
    }
}

/// RES-091: the original dispatch loop, factored out so `run` can
/// wrap any returned error with source-line info. `last_pc` is
/// updated at the top of every iteration so the outer wrapper knows
/// which instruction was about to execute when the failure fired.
fn run_inner(
    program: &Program,
    last_pc: &mut (usize, usize),
) -> Result<Value, VmError> {
    let mut stack: Vec<Value> = Vec::with_capacity(64);
    let mut locals: Vec<Value> = Vec::new();
    let mut frames: Vec<CallFrame> = Vec::with_capacity(16);
    frames.push(CallFrame {
        chunk_idx: usize::MAX, // main
        pc: 0,
        locals_base: 0,
    });

    loop {
        // SAFETY: frames is non-empty for the duration of the main
        // loop — we only pop a frame on `ReturnFromCall` (which has
        // a pre-check) and exit the loop on main's `Op::Return`.
        let frame_idx = frames.len() - 1;
        let (chunk, pc) = {
            let f = &frames[frame_idx];
            let chunk: &Chunk = if f.chunk_idx == usize::MAX {
                &program.main
            } else {
                &program.functions[f.chunk_idx].chunk
            };
            (chunk, f.pc)
        };
        // RES-091: snapshot which (chunk, pc) is about to be
        // attempted. After pc-advance we add 1, so err_at's
        // `saturating_sub(1)` lands back on this op.
        *last_pc = (frames[frame_idx].chunk_idx, pc + 1);
        if pc >= chunk.code.len() {
            // Ran off the end without an explicit return. Treat as
            // an implicit Return / ReturnFromCall depending on
            // whether we're in main.
            if frames.len() == 1 {
                return Ok(stack.pop().unwrap_or(Value::Void));
            }
            // In a fn body: implicit ReturnFromCall with Void.
            let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
            locals.truncate(popped.locals_base);
            stack.push(Value::Void);
            continue;
        }
        let op = chunk.code[pc];
        frames[frame_idx].pc += 1;

        match op {
            Op::Const(idx) => {
                let v = chunk
                    .constants
                    .get(idx as usize)
                    .ok_or(VmError::ConstantOutOfBounds(idx))?
                    .clone();
                stack.push(v);
            }
            Op::Add => {
                let (a, b) = pop_two_ints(&mut stack, "Add")?;
                stack.push(Value::Int(a.wrapping_add(b)));
            }
            Op::Sub => {
                let (a, b) = pop_two_ints(&mut stack, "Sub")?;
                stack.push(Value::Int(a.wrapping_sub(b)));
            }
            Op::Mul => {
                let (a, b) = pop_two_ints(&mut stack, "Mul")?;
                stack.push(Value::Int(a.wrapping_mul(b)));
            }
            Op::Div => {
                let (a, b) = pop_two_ints(&mut stack, "Div")?;
                if b == 0 {
                    return Err(VmError::DivideByZero);
                }
                stack.push(Value::Int(a / b));
            }
            Op::Mod => {
                let (a, b) = pop_two_ints(&mut stack, "Mod")?;
                if b == 0 {
                    return Err(VmError::DivideByZero);
                }
                stack.push(Value::Int(a % b));
            }
            Op::Neg => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Int(i) = v else {
                    return Err(VmError::TypeMismatch("Neg"));
                };
                stack.push(Value::Int(i.wrapping_neg()));
            }
            Op::LoadLocal(idx) => {
                let base = frames[frame_idx].locals_base;
                let abs = base + idx as usize;
                let v = locals
                    .get(abs)
                    .ok_or(VmError::LocalOutOfBounds(idx))?
                    .clone();
                stack.push(v);
            }
            Op::StoreLocal(idx) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let base = frames[frame_idx].locals_base;
                let abs = base + idx as usize;
                if locals.len() <= abs {
                    locals.resize(abs + 1, Value::Void);
                }
                locals[abs] = v;
            }
            Op::Call(idx) => {
                // RES-081: set up a fresh call frame. Pop `arity`
                // values as args (leftmost arg is the deepest push),
                // reserve `local_count` slots in the locals slab
                // (params plus body-local bindings), copy args into
                // slots 0..arity.
                let func = program
                    .functions
                    .get(idx as usize)
                    .ok_or(VmError::FunctionOutOfBounds(idx))?;
                let arity = func.arity as usize;
                if stack.len() < arity {
                    return Err(VmError::EmptyStack);
                }
                if frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let base = locals.len();
                // Reserve the full locals slab for the callee up
                // front, then overwrite the first `arity` slots with
                // args. Popping from the stack gives rightmost arg
                // first, so write backwards.
                locals.resize(base + func.local_count as usize, Value::Void);
                for i in (0..arity).rev() {
                    let v = stack.pop().ok_or(VmError::EmptyStack)?;
                    locals[base + i] = v;
                }
                frames.push(CallFrame {
                    chunk_idx: idx as usize,
                    pc: 0,
                    locals_base: base,
                });
            }
            Op::ReturnFromCall => {
                // Pop the return value, unwind the frame, push it
                // onto the caller's stack.
                let ret = stack.pop().ok_or(VmError::EmptyStack)?;
                let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
                if frames.is_empty() {
                    // ReturnFromCall at top level — shouldn't happen
                    // for well-formed programs. Treat as halt so
                    // hand-rolled chunks don't panic.
                    return Ok(ret);
                }
                locals.truncate(popped.locals_base);
                stack.push(ret);
            }
            Op::Jump(offset) => {
                let new_pc = (frames[frame_idx].pc as isize) + offset as isize;
                if new_pc < 0 || (new_pc as usize) > chunk.code.len() {
                    return Err(VmError::JumpOutOfBounds);
                }
                frames[frame_idx].pc = new_pc as usize;
            }
            Op::JumpIfFalse(offset) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let is_falsy = match v {
                    Value::Bool(b) => !b,
                    Value::Int(i) => i == 0,
                    _ => return Err(VmError::TypeMismatch("JumpIfFalse")),
                };
                if is_falsy {
                    let new_pc = (frames[frame_idx].pc as isize) + offset as isize;
                    if new_pc < 0 || (new_pc as usize) > chunk.code.len() {
                        return Err(VmError::JumpOutOfBounds);
                    }
                    frames[frame_idx].pc = new_pc as usize;
                }
            }
            Op::Eq => {
                let (a, b) = pop_two_ints(&mut stack, "Eq")?;
                stack.push(Value::Bool(a == b));
            }
            Op::Neq => {
                let (a, b) = pop_two_ints(&mut stack, "Neq")?;
                stack.push(Value::Bool(a != b));
            }
            Op::Lt => {
                let (a, b) = pop_two_ints(&mut stack, "Lt")?;
                stack.push(Value::Bool(a < b));
            }
            Op::Le => {
                let (a, b) = pop_two_ints(&mut stack, "Le")?;
                stack.push(Value::Bool(a <= b));
            }
            Op::Gt => {
                let (a, b) = pop_two_ints(&mut stack, "Gt")?;
                stack.push(Value::Bool(a > b));
            }
            Op::Ge => {
                let (a, b) = pop_two_ints(&mut stack, "Ge")?;
                stack.push(Value::Bool(a >= b));
            }
            Op::Not => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Bool(b) = v else {
                    return Err(VmError::TypeMismatch("Not"));
                };
                stack.push(Value::Bool(!b));
            }
            Op::Return => {
                return Ok(stack.pop().unwrap_or(Value::Void));
            }
        }
    }
}

fn pop_two_ints(stack: &mut Vec<Value>, op_name: &'static str) -> Result<(i64, i64), VmError> {
    let b = stack.pop().ok_or(VmError::EmptyStack)?;
    let a = stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Ok((x, y)),
        _ => Err(VmError::TypeMismatch(op_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Op, Program};

    fn const_program(values: &[Value], code: &[Op]) -> Program {
        let mut main = Chunk::new();
        for v in values {
            main.constants.push(v.clone());
        }
        for op in code {
            main.code.push(*op);
            main.line_info.push(1);
        }
        Program { main, functions: Vec::new() }
    }

    fn compile_run(src: &str) -> Result<Value, VmError> {
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        run(&prog)
    }

    fn assert_int(actual: Value, expected: i64) {
        match actual {
            Value::Int(v) => assert_eq!(v, expected, "expected Int({}), got Int({})", expected, v),
            other => panic!("expected Int({}), got {:?}", expected, other),
        }
    }

    #[test]
    fn const_then_return_yields_value() {
        let p = const_program(&[Value::Int(7)], &[Op::Const(0), Op::Return]);
        assert_int(run(&p).unwrap(), 7);
    }

    #[test]
    fn add_two_ints() {
        let p = const_program(
            &[Value::Int(2), Value::Int(3)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        assert_int(run(&p).unwrap(), 5);
    }

    #[test]
    fn end_to_end_two_plus_three_times_four() {
        assert_int(compile_run("2 + 3 * 4;").unwrap(), 14);
    }

    #[test]
    fn let_then_load_yields_stored_value() {
        assert_int(compile_run("let x = 9; x;").unwrap(), 9);
    }

    #[test]
    fn divide_by_zero_inside_fn_body_attributes_to_body_line() {
        // RES-092: the divide-by-zero is on the SECOND line of source
        // (inside the fn body). Without RES-092 this would land at
        // line 1 (the `fn` declaration).
        let src = "fn unsafe_div(int n) {\n    let r = 100 / n;\n    return r;\n}\nunsafe_div(0);";
        let err = compile_run(src).unwrap_err();
        let display = err.to_string();
        assert!(display.contains("divide by zero"), "missing kind: {}", display);
        assert!(
            display.contains("line 2"),
            "expected `line 2` (the body's divide line); got: {}",
            display
        );
    }

    #[test]
    fn divide_by_zero_error_includes_source_line() {
        // RES-091: the runtime error wraps the underlying kind with
        // VmError::AtLine carrying the source line. Display should
        // print `(line N)` suffix.
        let err = compile_run("let x = 10 / 0;").unwrap_err();
        let display = err.to_string();
        assert!(
            display.contains("divide by zero"),
            "missing divide-by-zero text: {}",
            display
        );
        assert!(
            display.contains("line "),
            "missing line attribution: {}",
            display
        );
        // The kind() helper still returns the raw variant for tests
        // that match on kind.
        assert_eq!(err.kind(), &VmError::DivideByZero);
    }

    #[test]
    fn divide_by_zero_is_clean_error() {
        // RES-091: errors are now wrapped with line info, so compare
        // on the inner kind via VmError::kind().
        let err = compile_run("10 / 0;").unwrap_err();
        assert_eq!(err.kind(), &VmError::DivideByZero);
    }

    #[test]
    fn type_mismatch_on_add_with_string_constant() {
        let p = const_program(
            &[Value::Int(1), Value::String("x".into())],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        let err = run(&p).unwrap_err();
        assert_eq!(err.kind(), &VmError::TypeMismatch("Add"));
    }

    #[test]
    fn negation_works() {
        assert_int(compile_run("let x = -7; x;").unwrap(), -7);
    }

    // ---------- RES-081 tests ----------

    #[test]
    fn zero_arg_function_call_returns_its_constant() {
        let src = "fn zero() { return 0; } zero();";
        assert_int(compile_run(src).unwrap(), 0);
    }

    #[test]
    fn unary_function_squares_its_argument() {
        let src = "fn sq(int n) { return n * n; } sq(5);";
        assert_int(compile_run(src).unwrap(), 25);
    }

    #[test]
    fn two_arg_function_adds_its_arguments() {
        let src = "fn add(int a, int b) { return a + b; } add(3, 4);";
        assert_int(compile_run(src).unwrap(), 7);
    }

    #[test]
    fn fn_arg_order_is_source_order() {
        // a - b is order-sensitive: sub(10, 3) = 7, sub(3, 10) = -7.
        let src = "fn sub(int a, int b) { return a - b; } sub(10, 3);";
        assert_int(compile_run(src).unwrap(), 7);
    }

    #[test]
    fn fn_with_let_in_body_works() {
        // Body uses a local beyond the param slots.
        let src = "fn work(int n) { let doubled = n + n; return doubled + 1; } work(5);";
        assert_int(compile_run(src).unwrap(), 11);
    }

    #[test]
    fn call_stack_overflow_on_runaway_recursion() {
        // Without RES-083's `if`, we can't write terminating
        // recursion in source yet. Hand-roll a chunk whose body
        // is just `Call(self); ReturnFromCall` — blows the stack.
        use crate::bytecode::Function;
        let mut body = Chunk::new();
        body.code.push(Op::Call(0));
        body.code.push(Op::ReturnFromCall);
        body.line_info.push(1);
        body.line_info.push(1);
        let runaway = Function {
            name: "runaway".into(),
            arity: 0,
            chunk: body,
            local_count: 0,
        };
        let mut main = Chunk::new();
        main.code.push(Op::Call(0));
        main.code.push(Op::Return);
        main.line_info.push(1);
        main.line_info.push(1);
        let p = Program { main, functions: vec![runaway] };
        let err = run(&p).unwrap_err();
        assert_eq!(err.kind(), &VmError::CallStackOverflow);
    }

    // ---------- RES-083 tests ----------

    #[test]
    fn if_true_picks_consequence() {
        assert_int(compile_run("if true { 1; } else { 2; }").unwrap(), 1);
    }

    #[test]
    fn if_false_picks_alternative() {
        assert_int(compile_run("if false { 1; } else { 2; }").unwrap(), 2);
    }

    #[test]
    fn if_without_else_and_false_cond_leaves_void() {
        // No value pushed — top-level Return sees an empty stack.
        let result = compile_run("if false { let x = 1; }").unwrap();
        assert!(matches!(result, Value::Void), "got {:?}", result);
    }

    #[test]
    fn while_counting_loop_accumulates() {
        // i=0; sum=0; while i<5 { sum=sum+i; i=i+1; } sum;  →  0+1+2+3+4 = 10
        let src = "let i = 0; let sum = 0; while i < 5 { sum = sum + i; i = i + 1; } sum;";
        assert_int(compile_run(src).unwrap(), 10);
    }

    #[test]
    fn recursive_fib_ten_is_fifty_five() {
        // The payoff test — recursion + branching together.
        let src = "fn fib(int n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fib(10);";
        assert_int(compile_run(src).unwrap(), 55);
    }

    #[test]
    fn comparison_ops_produce_bool() {
        // Use `if` to inspect the comparison result — we don't have
        // a public Bool probe, but `if 3 < 5 { 1; } else { 0; }` tells
        // us 1 iff Lt evaluated to true.
        assert_int(compile_run("if 3 < 5 { 1; } else { 0; }").unwrap(), 1);
        assert_int(compile_run("if 5 < 3 { 1; } else { 0; }").unwrap(), 0);
        assert_int(compile_run("if 5 == 5 { 1; } else { 0; }").unwrap(), 1);
        assert_int(compile_run("if 5 != 5 { 1; } else { 0; }").unwrap(), 0);
    }

    #[test]
    fn logical_and_short_circuits() {
        // `false && <anything>` evaluates to false without evaluating rhs.
        // We can't directly observe short-circuit without side effects,
        // but we can at least confirm the result shape matches for
        // both paths.
        assert_int(
            compile_run("if true && true { 1; } else { 0; }").unwrap(),
            1,
        );
        assert_int(
            compile_run("if true && false { 1; } else { 0; }").unwrap(),
            0,
        );
        assert_int(
            compile_run("if false && true { 1; } else { 0; }").unwrap(),
            0,
        );
    }

    #[test]
    fn logical_or_short_circuits() {
        assert_int(
            compile_run("if true || false { 1; } else { 0; }").unwrap(),
            1,
        );
        assert_int(
            compile_run("if false || true { 1; } else { 0; }").unwrap(),
            1,
        );
        assert_int(
            compile_run("if false || false { 1; } else { 0; }").unwrap(),
            0,
        );
    }

    #[test]
    fn not_negates_boolean() {
        assert_int(compile_run("if !false { 1; } else { 0; }").unwrap(), 1);
        assert_int(compile_run("if !true { 1; } else { 0; }").unwrap(), 0);
    }

    #[test]
    fn for_in_is_still_unsupported() {
        // RES-083 explicitly scoped `for-in` out.
        let (program, _) = crate::parse("for x in [1,2,3] { let y = x; }");
        let err = crate::compiler::compile(&program).unwrap_err();
        assert!(
            matches!(err, crate::bytecode::CompileError::Unsupported(_)),
            "{:?}",
            err
        );
    }

    #[test]
    fn vm_and_tree_walker_agree_on_call_result() {
        // Oracle check: for a program both paths accept, the VM and
        // the interpreter must return the same value.
        let src = "fn sq(int n) { return n * n; } sq(6);";
        let (ast, _) = crate::parse(src);
        let prog = crate::compiler::compile(&ast).unwrap();
        let vm_result = run(&prog).unwrap();

        // Tree walker: eval the whole program, then look up main
        // return by evaluating the call manually via a fresh
        // interpreter.
        let mut interp = crate::Interpreter::new();
        // Eval to register the fn.
        interp.eval(&ast).unwrap();
        // Then invoke sq(6) as a standalone call expression.
        let (call_ast, _) = crate::parse("sq(6);");
        // Hoist the program fns into interp first (done above) then
        // evaluate the call expression.
        let Value::Int(interp_val) = eval_first_stmt(&mut interp, &call_ast) else {
            panic!("interpreter didn't return Int for sq(6)");
        };
        let Value::Int(vm_val) = vm_result else {
            panic!("VM didn't return Int");
        };
        assert_eq!(interp_val, vm_val);
    }

    /// Evaluate the first top-level statement of `program` in the
    /// given interpreter, returning the resulting value.
    fn eval_first_stmt(interp: &mut crate::Interpreter, program: &crate::Node) -> Value {
        let crate::Node::Program(stmts) = program else {
            panic!("expected Program");
        };
        interp.eval(&stmts[0].node).expect("eval")
    }
}
