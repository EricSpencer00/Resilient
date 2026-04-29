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

use crate::Value;
use crate::bytecode::{Chunk, Op, Program};

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
    /// RES-169a: an opcode was dispatched that the VM recognizes as
    /// a valid variant but hasn't wired runtime semantics for yet.
    /// Currently used as the placeholder dispatch arm for
    /// `Op::MakeClosure` and `Op::LoadUpvalue` — the compiler never
    /// emits these until RES-169b lands.
    Unsupported(&'static str),
    /// RES-171a: array indexing ran past the array's length (or
    /// got a negative index). Carries the offending index and the
    /// array's length so the Display message is diagnostic-ready.
    ArrayIndexOutOfBounds {
        index: i64,
        len: usize,
    },
    /// FFI v2: the trampoline returned an error string.
    ForeignCallFailed(String),
    /// RES-VM (issue #266): a `CallBuiltin` op referenced a name that
    /// isn't present in the canonical `BUILTINS` table. Should not
    /// happen for chunks emitted by `compiler::compile` (which only
    /// emits `CallBuiltin` after a positive lookup); guards hand-rolled
    /// chunks and protects against a builtin being removed from the
    /// table without recompiling cached bytecode.
    UnknownBuiltin(String),
    /// RES-VM (issue #266): the builtin function returned an `Err`.
    /// Wraps the message it produced so the user sees the same
    /// diagnostic the tree-walker would have emitted.
    BuiltinCallFailed(String),
    /// RES-335: `GetField`/`SetField` on a struct value whose field
    /// table has no entry matching the requested name.
    UnknownField {
        struct_name: String,
        field: String,
    },
    /// RES-349: integer arithmetic overflowed under `OverflowMode::Trap`.
    /// Carries a static label naming the offending op (`"Add"`, `"Sub"`,
    /// `"Mul"`, `"Neg"`, `"IncLocal"`) so diagnostics are precise.
    IntegerOverflow(&'static str),
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
            VmError::Unsupported(what) => write!(f, "vm: unsupported opcode: {}", what),
            VmError::ArrayIndexOutOfBounds { index, len } => write!(
                f,
                "vm: array index {} out of bounds for length {}",
                index, len
            ),
            VmError::AtLine { line, kind } => write!(f, "{} (line {})", kind, line),
            VmError::ForeignCallFailed(msg) => write!(f, "ffi call failed: {}", msg),
            VmError::UnknownBuiltin(name) => write!(f, "vm: unknown builtin: {}", name),
            VmError::BuiltinCallFailed(msg) => write!(f, "vm: builtin call failed: {}", msg),
            VmError::UnknownField { struct_name, field } => {
                write!(f, "vm: struct {} has no field '{}'", struct_name, field)
            }
            VmError::IntegerOverflow(what) => {
                write!(f, "vm: integer overflow in {}", what)
            }
        }
    }
}

impl std::error::Error for VmError {}

/// RES-349: overflow behaviour for integer arithmetic (`Add`, `Sub`,
/// `Mul`, `Neg`, and the `IncLocal` peephole).
///
/// Selected by the `RESILIENT_OVERFLOW_MODE` environment variable
/// (`wrap`, `saturate`, `trap`). The default is [`OverflowMode::Wrap`]
/// — two's-complement wraparound — which preserves the byte-identical
/// behaviour the VM has shipped since RES-076. Crypto/hashing code
/// expects `Wrap`; safety-critical code typically wants `Trap`.
///
/// The mode is read once per `run` call, so changing the env var
/// mid-program has no effect (programs that need per-block control
/// can set it before invoking the VM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowMode {
    /// Two's-complement wraparound. Default. Matches `i64::wrapping_*`.
    Wrap,
    /// Clamp to `i64::MIN` / `i64::MAX` on overflow.
    Saturate,
    /// Surface a [`VmError::IntegerOverflow`] runtime error.
    Trap,
}

impl OverflowMode {
    /// Read `RESILIENT_OVERFLOW_MODE` and return the configured mode.
    /// Unknown / unset values fall back to [`OverflowMode::Wrap`] so
    /// behaviour stays byte-identical with pre-RES-349 builds.
    pub fn from_env() -> Self {
        match std::env::var("RESILIENT_OVERFLOW_MODE")
            .as_deref()
            .map(str::trim)
        {
            Ok("saturate") | Ok("Saturate") | Ok("SATURATE") => Self::Saturate,
            Ok("trap") | Ok("Trap") | Ok("TRAP") => Self::Trap,
            _ => Self::Wrap,
        }
    }

    fn add(self, a: i64, b: i64, label: &'static str) -> Result<i64, VmError> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_add(b)),
            OverflowMode::Saturate => Ok(a.saturating_add(b)),
            OverflowMode::Trap => a.checked_add(b).ok_or(VmError::IntegerOverflow(label)),
        }
    }

    fn sub(self, a: i64, b: i64, label: &'static str) -> Result<i64, VmError> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_sub(b)),
            OverflowMode::Saturate => Ok(a.saturating_sub(b)),
            OverflowMode::Trap => a.checked_sub(b).ok_or(VmError::IntegerOverflow(label)),
        }
    }

    fn mul(self, a: i64, b: i64, label: &'static str) -> Result<i64, VmError> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_mul(b)),
            OverflowMode::Saturate => Ok(a.saturating_mul(b)),
            OverflowMode::Trap => a.checked_mul(b).ok_or(VmError::IntegerOverflow(label)),
        }
    }

    fn neg(self, a: i64, label: &'static str) -> Result<i64, VmError> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_neg()),
            // i64::MIN.saturating_neg() returns i64::MAX, the documented behaviour.
            OverflowMode::Saturate => Ok(a.saturating_neg()),
            OverflowMode::Trap => a.checked_neg().ok_or(VmError::IntegerOverflow(label)),
        }
    }

    /// RES-349: tree-walker variant. The interpreter in `main.rs`
    /// reports errors as `String`, not `VmError`, so the trap path
    /// formats a matching diagnostic.
    pub fn add_for_eval(self, a: i64, b: i64, op: &str) -> Result<i64, String> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_add(b)),
            OverflowMode::Saturate => Ok(a.saturating_add(b)),
            OverflowMode::Trap => a
                .checked_add(b)
                .ok_or_else(|| format!("integer overflow in {} ({} {} {})", op, a, op, b)),
        }
    }

    /// RES-349: tree-walker variant of [`OverflowMode::sub`].
    pub fn sub_for_eval(self, a: i64, b: i64, op: &str) -> Result<i64, String> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_sub(b)),
            OverflowMode::Saturate => Ok(a.saturating_sub(b)),
            OverflowMode::Trap => a
                .checked_sub(b)
                .ok_or_else(|| format!("integer overflow in {} ({} {} {})", op, a, op, b)),
        }
    }

    /// RES-349: tree-walker variant of [`OverflowMode::mul`].
    pub fn mul_for_eval(self, a: i64, b: i64, op: &str) -> Result<i64, String> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_mul(b)),
            OverflowMode::Saturate => Ok(a.saturating_mul(b)),
            OverflowMode::Trap => a
                .checked_mul(b)
                .ok_or_else(|| format!("integer overflow in {} ({} {} {})", op, a, op, b)),
        }
    }

    /// RES-349: tree-walker variant of [`OverflowMode::neg`].
    pub fn neg_for_eval(self, a: i64) -> Result<i64, String> {
        match self {
            OverflowMode::Wrap => Ok(a.wrapping_neg()),
            OverflowMode::Saturate => Ok(a.saturating_neg()),
            OverflowMode::Trap => a
                .checked_neg()
                .ok_or_else(|| format!("integer overflow in unary - ({})", a)),
        }
    }
}

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
        Some(&line) if line > 0 => VmError::AtLine {
            line,
            kind: Box::new(e),
        },
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

/// RES-329: dispatch strategy. The default match-based loop and the
/// direct-threaded function-pointer table produce byte-identical
/// results; the threaded path is selected with `RESILIENT_DISPATCH=direct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// Centralized `match op { ... }` loop (default, pre-RES-329 behaviour).
    Match,
    /// Direct-threaded: each opcode has its own handler `fn(&mut VmState, Op) -> Result<Step, VmError>`,
    /// indexed by opcode discriminant. Removes the central switch's
    /// branch-prediction pressure on instruction-dense loops.
    Direct,
}

impl Dispatch {
    /// Read `RESILIENT_DISPATCH` and return the configured strategy.
    /// Unknown / unset values fall back to [`Dispatch::Match`] so default
    /// behaviour stays byte-identical with pre-RES-329 builds.
    pub fn from_env() -> Self {
        match std::env::var("RESILIENT_DISPATCH")
            .as_deref()
            .map(str::trim)
        {
            Ok("direct") | Ok("Direct") | Ok("DIRECT") | Ok("threaded") => Self::Direct,
            _ => Self::Match,
        }
    }
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
    run_with_mode(program, OverflowMode::from_env())
}

/// RES-349: explicit-mode entry point. The default `run` reads the
/// mode from `RESILIENT_OVERFLOW_MODE`; tests and embedders that need
/// to pin a mode without mutating process env can call this directly.
///
/// RES-329: also reads `RESILIENT_DISPATCH` to pick the dispatch
/// strategy. Both paths produce byte-identical results.
pub fn run_with_mode(program: &Program, mode: OverflowMode) -> Result<Value, VmError> {
    run_with(program, mode, Dispatch::from_env())
}

/// RES-329: fully-explicit entry point. Tests and benchmarks pick the
/// dispatch strategy directly without mutating process env.
pub fn run_with(
    program: &Program,
    mode: OverflowMode,
    dispatch: Dispatch,
) -> Result<Value, VmError> {
    // Sentinel for "no failure attributable yet" — main chunk @ pc 0.
    let mut last_pc: (usize, usize) = (usize::MAX, 0);
    let result = match dispatch {
        Dispatch::Match => run_inner(program, &mut last_pc, mode),
        Dispatch::Direct => run_direct(program, &mut last_pc, mode),
    };
    match result {
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
    overflow_mode: OverflowMode,
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
                stack.push(Value::Int(overflow_mode.add(a, b, "Add")?));
            }
            Op::Sub => {
                let (a, b) = pop_two_ints(&mut stack, "Sub")?;
                stack.push(Value::Int(overflow_mode.sub(a, b, "Sub")?));
            }
            Op::Mul => {
                let (a, b) = pop_two_ints(&mut stack, "Mul")?;
                stack.push(Value::Int(overflow_mode.mul(a, b, "Mul")?));
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
                stack.push(Value::Int(overflow_mode.neg(i, "Neg")?));
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
            // RES-384: self-tail-call. Reuse the current frame
            // instead of pushing a new one — O(1) call-stack depth
            // for tail-recursive functions. Steps:
            //   1. Look up the callee (must be the same function).
            //   2. Pop `arity` args off the operand stack.
            //   3. Overwrite locals[locals_base..locals_base+arity]
            //      with the new args (in source order).
            //   4. Reset frame.pc to 0 so the next iteration
            //      starts the function from the top.
            // We do NOT push a new CallFrame — the existing frame is
            // reused with its locals slab base unchanged.
            Op::TailCall(idx) => {
                let func = program
                    .functions
                    .get(idx as usize)
                    .ok_or(VmError::FunctionOutOfBounds(idx))?;
                let arity = func.arity as usize;
                if stack.len() < arity {
                    return Err(VmError::EmptyStack);
                }
                // The locals slab for this frame may be larger than
                // arity (body has let-bindings beyond params). We
                // only reset the parameter slots; body-locals are
                // re-initialized by StoreLocal on the next pass.
                let base = frames[frame_idx].locals_base;
                // Pop args in reverse order (rightmost first) and
                // write into locals[base+0..base+arity] in source
                // order, matching the Call path.
                for i in (0..arity).rev() {
                    let v = stack.pop().ok_or(VmError::EmptyStack)?;
                    locals[base + i] = v;
                }
                // Reset pc: the next loop iteration picks up at
                // instruction 0 of this frame's chunk.
                frames[frame_idx].pc = 0;
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
            // RES-172: inverse of JumpIfFalse. Peephole emits this
            // for `Not; JumpIfFalse(off)` folds.
            Op::JumpIfTrue(offset) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let is_truthy = match v {
                    Value::Bool(b) => b,
                    Value::Int(i) => i != 0,
                    _ => return Err(VmError::TypeMismatch("JumpIfTrue")),
                };
                if is_truthy {
                    let new_pc = (frames[frame_idx].pc as isize) + offset as isize;
                    if new_pc < 0 || (new_pc as usize) > chunk.code.len() {
                        return Err(VmError::JumpOutOfBounds);
                    }
                    frames[frame_idx].pc = new_pc as usize;
                }
            }
            // RES-172: in-place local increment. The peephole emits
            // this for `LoadLocal x; Const 1; Add; StoreLocal x`
            // idioms, saving three ops + the stack churn of the
            // fold.
            Op::IncLocal(idx) => {
                let base = frames[frame_idx].locals_base;
                let abs = base + idx as usize;
                let v = locals.get(abs).ok_or(VmError::LocalOutOfBounds(idx))?;
                let Value::Int(n) = *v else {
                    return Err(VmError::TypeMismatch("IncLocal"));
                };
                locals[abs] = Value::Int(overflow_mode.add(n, 1, "IncLocal")?);
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
            // RES-169a: skeleton dispatch arms. The compiler never
            // emits these until RES-169b lands the MakeClosure /
            // LoadUpvalue emission pass; if one shows up in a chunk
            // today it's a wiring bug, not user-facing. Return
            // Unsupported with a self-describing descriptor so the
            // at-line wrapper still works.
            Op::MakeClosure { .. } => {
                return Err(VmError::Unsupported("MakeClosure"));
            }
            Op::LoadUpvalue(_) => {
                return Err(VmError::Unsupported("LoadUpvalue"));
            }
            #[cfg(feature = "ffi")]
            Op::CallForeign(idx) => {
                let sym = program
                    .foreign_syms
                    .get(idx as usize)
                    .ok_or(VmError::FunctionOutOfBounds(idx))?;
                let arity = sym.sig.params.len();
                if stack.len() < arity {
                    return Err(VmError::EmptyStack);
                }
                // Args were pushed left-to-right; pop rightmost first, then reverse.
                let mut args: Vec<crate::Value> = (0..arity)
                    .map(|_| stack.pop().expect("checked above"))
                    .collect();
                args.reverse();
                let result = crate::ffi_trampolines::call_foreign(sym, &args)
                    .map_err(VmError::ForeignCallFailed)?;
                stack.push(result);
            }
            #[cfg(not(feature = "ffi"))]
            Op::CallForeign(_) => {
                return Err(VmError::Unsupported(
                    "CallForeign (build without --features ffi)",
                ));
            }
            // RES-VM (issue #266): builtin call. Resolve the name from
            // the constant pool, look it up in the canonical BUILTINS
            // table, pop `arity` arguments (reversing into source
            // order), invoke the function, and push the result. The
            // builtin returns `RResult<Value>` (`Result<Value, String>`);
            // we wrap the error string in `BuiltinCallFailed`.
            Op::CallBuiltin { name_const, arity } => {
                let name_val = chunk
                    .constants
                    .get(name_const as usize)
                    .ok_or(VmError::ConstantOutOfBounds(name_const))?;
                let name = match name_val {
                    Value::String(s) => s.clone(),
                    _ => return Err(VmError::TypeMismatch("CallBuiltin (non-string name)")),
                };
                let func = crate::lookup_builtin(&name)
                    .ok_or_else(|| VmError::UnknownBuiltin(name.clone()))?;
                let n = arity as usize;
                if stack.len() < n {
                    return Err(VmError::EmptyStack);
                }
                let mut args: Vec<Value> = (0..n)
                    .map(|_| stack.pop().expect("checked above"))
                    .collect();
                args.reverse();
                let result = func(&args).map_err(VmError::BuiltinCallFailed)?;
                stack.push(result);
            }
            // ---- RES-171a: array ops ----
            Op::MakeArray { len } => {
                // Pop `len` values. The source literal `[a, b, c]`
                // pushes a, b, c in order, so the bottom-most one
                // on the popped-span is `a`. Use `stack.drain` to
                // pull a contiguous range without cloning.
                let n = len as usize;
                if stack.len() < n {
                    return Err(VmError::EmptyStack);
                }
                let split_at = stack.len() - n;
                let items: Vec<Value> = stack.drain(split_at..).collect();
                stack.push(Value::Array(items));
            }
            Op::LoadIndex => {
                let idx_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let arr_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Int(idx) = idx_val else {
                    return Err(VmError::TypeMismatch("LoadIndex (non-int index)"));
                };
                let Value::Array(items) = arr_val else {
                    return Err(VmError::TypeMismatch("LoadIndex (non-array target)"));
                };
                if idx < 0 || (idx as usize) >= items.len() {
                    return Err(VmError::ArrayIndexOutOfBounds {
                        index: idx,
                        len: items.len(),
                    });
                }
                stack.push(items[idx as usize].clone());
            }
            // RES-407: emitted only when `bounds_check::check_array_bounds`
            // discharged the bounds obligation for this site. Skips the
            // bounds check; type checks on operands stay.
            Op::LoadIndexUnchecked => {
                let idx_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let arr_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Int(idx) = idx_val else {
                    return Err(VmError::TypeMismatch("LoadIndexUnchecked (non-int index)"));
                };
                let Value::Array(items) = arr_val else {
                    return Err(VmError::TypeMismatch(
                        "LoadIndexUnchecked (non-array target)",
                    ));
                };
                stack.push(items[idx as usize].clone());
            }
            Op::StoreIndex => {
                // Stack layout on entry (top → bottom):
                //   [v, idx, arr, ...]
                // Pop in reverse-push order.
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let idx_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let arr_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Int(idx) = idx_val else {
                    return Err(VmError::TypeMismatch("StoreIndex (non-int index)"));
                };
                let Value::Array(mut items) = arr_val else {
                    return Err(VmError::TypeMismatch("StoreIndex (non-array target)"));
                };
                if idx < 0 || (idx as usize) >= items.len() {
                    return Err(VmError::ArrayIndexOutOfBounds {
                        index: idx,
                        len: items.len(),
                    });
                }
                items[idx as usize] = v;
                // Push the modified array back so the enclosing
                // compile pattern (`StoreLocal` after `StoreIndex`)
                // can write it into the local slot.
                stack.push(Value::Array(items));
            }
            // ---- RES-335: struct ops ----
            Op::StructLiteral {
                name_const,
                field_count,
            } => {
                let name = constant_as_string(chunk, name_const, "StructLiteral (type name)")?;
                let n = field_count as usize;
                // Stack layout on entry (top → bottom):
                //   [v_N, k_N, ..., v_1, k_1, ...]
                // Drain the top `2 * n` values as a flat vec and
                // destructure into (key, value) pairs in push order.
                let needed = n.checked_mul(2).ok_or(VmError::EmptyStack)?;
                if stack.len() < needed {
                    return Err(VmError::EmptyStack);
                }
                let split_at = stack.len() - needed;
                let flat: Vec<Value> = stack.drain(split_at..).collect();
                let mut fields: Vec<(String, Value)> = Vec::with_capacity(n);
                let mut it = flat.into_iter();
                for _ in 0..n {
                    let k = it.next().ok_or(VmError::EmptyStack)?;
                    let v = it.next().ok_or(VmError::EmptyStack)?;
                    let Value::String(field_name) = k else {
                        return Err(VmError::TypeMismatch("StructLiteral (non-string key)"));
                    };
                    fields.push((field_name, v));
                }
                stack.push(Value::Struct { name, fields });
            }
            Op::GetField { name_const } => {
                let field = constant_as_string(chunk, name_const, "GetField (field name)")?;
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Struct {
                    name: sname,
                    fields,
                } = v
                else {
                    return Err(VmError::TypeMismatch("GetField (non-struct target)"));
                };
                let found = fields
                    .iter()
                    .find(|(k, _)| k == &field)
                    .map(|(_, v)| v.clone());
                match found {
                    Some(val) => stack.push(val),
                    None => {
                        return Err(VmError::UnknownField {
                            struct_name: sname,
                            field,
                        });
                    }
                }
            }
            Op::SetField { name_const } => {
                let field = constant_as_string(chunk, name_const, "SetField (field name)")?;
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let tgt = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Struct {
                    name: sname,
                    mut fields,
                } = tgt
                else {
                    return Err(VmError::TypeMismatch("SetField (non-struct target)"));
                };
                let slot = fields.iter_mut().find(|(k, _)| k == &field);
                match slot {
                    Some((_, existing)) => *existing = v,
                    None => {
                        return Err(VmError::UnknownField {
                            struct_name: sname,
                            field,
                        });
                    }
                }
                // Push the modified struct back so the enclosing
                // compile pattern (`StoreLocal` after `SetField`) can
                // write it into the local slot. Mirrors `StoreIndex`.
                stack.push(Value::Struct {
                    name: sname,
                    fields,
                });
            }
        }
    }
}

/// RES-335: pull a `Value::String` out of `chunk.constants[idx]` or
/// surface a type-shaped VM error. Used by the struct opcodes, all of
/// which carry field/type names via the constant pool to keep
/// `Op: Copy`.
fn constant_as_string(chunk: &Chunk, idx: u16, context: &'static str) -> Result<String, VmError> {
    let v = chunk
        .constants
        .get(idx as usize)
        .ok_or(VmError::ConstantOutOfBounds(idx))?;
    match v {
        Value::String(s) => Ok(s.clone()),
        _ => Err(VmError::TypeMismatch(context)),
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

// =============================================================================
// RES-329: direct-threaded dispatch
// =============================================================================
//
// The default dispatch is the centralized `match op { ... }` loop above.
// Direct threading replaces that single hot match with a per-opcode handler
// table indexed by `Op` discriminant. Each handler is a small function that
// owns its own branch-prediction history, removing the central switch's
// dispatch hazard on instruction-dense loops (e.g. tight `while` bodies,
// recursive arithmetic).
//
// This is the stable-Rust equivalent of computed-goto / threaded-code: there
// are no `unsafe` blocks and no nightly features. The function-pointer call
// is indirect (`call rax`-style on x86_64), but the branch predictor sees
// each handler as an independent dispatch site, which performs much better
// than the central switch on hot loops.
//
// Both dispatch paths share the same opcode semantics; the threaded module
// reuses `pop_two_ints`, `constant_as_string`, `err_at`, and `OverflowMode`.

/// Outcome of executing a single op in the direct-threaded path.
#[derive(Debug)]
enum Step {
    /// Continue dispatching from the new (frame, pc).
    Continue,
    /// Halt execution and yield the supplied value as the program's result.
    /// Emitted by `Op::Return` and by implicit-return-from-main.
    Halt(Value),
}

/// Mutable VM state threaded through every direct-dispatch handler.
/// Bundling the mutable parts into one struct keeps the handler signature
/// small (`fn(&mut VmState, Op) -> Result<Step, VmError>`), which the
/// optimizer can keep mostly in registers across handler boundaries.
struct VmState<'p> {
    program: &'p Program,
    stack: Vec<Value>,
    locals: Vec<Value>,
    frames: Vec<CallFrame>,
    overflow_mode: OverflowMode,
}

impl<'p> VmState<'p> {
    /// Reference to the chunk currently executing in the topmost frame.
    /// Inlined into every handler that touches the constant pool or the
    /// instruction stream.
    #[inline(always)]
    fn current_chunk(&self) -> &'p Chunk {
        let f = &self.frames[self.frames.len() - 1];
        if f.chunk_idx == usize::MAX {
            &self.program.main
        } else {
            &self.program.functions[f.chunk_idx].chunk
        }
    }

    #[inline(always)]
    fn frame_idx(&self) -> usize {
        self.frames.len() - 1
    }
}

/// Direct-threaded handler signature. Each handler executes one op,
/// mutating state, and reports whether to continue or halt.
type Handler = fn(&mut VmState<'_>, Op) -> Result<Step, VmError>;

/// Number of `Op` discriminants. Keep in sync with the `Op` enum in
/// `bytecode.rs`. The `op_to_index` table below pins the mapping; if a
/// new opcode is added, both `OP_KIND_COUNT` and the dispatch table must
/// grow together.
const OP_KIND_COUNT: usize = 32;

/// Map an `Op` to its dispatch-table index. Keeping this explicit (rather
/// than relying on `mem::discriminant` or transmute on the enum tag)
/// keeps the mapping stable across `repr` changes and makes the table
/// trivially auditable.
#[inline(always)]
fn op_to_index(op: Op) -> usize {
    match op {
        Op::Const(_) => 0,
        Op::Add => 1,
        Op::Sub => 2,
        Op::Mul => 3,
        Op::Div => 4,
        Op::Mod => 5,
        Op::Neg => 6,
        Op::LoadLocal(_) => 7,
        Op::StoreLocal(_) => 8,
        Op::Call(_) => 9,
        Op::ReturnFromCall => 10,
        Op::Jump(_) => 11,
        Op::JumpIfFalse(_) => 12,
        Op::JumpIfTrue(_) => 13,
        Op::IncLocal(_) => 14,
        Op::Eq => 15,
        Op::Neq => 16,
        Op::Lt => 17,
        Op::Le => 18,
        Op::Gt => 19,
        Op::Ge => 20,
        Op::Not => 21,
        Op::Return => 22,
        Op::MakeClosure { .. } => 23,
        Op::LoadUpvalue(_) => 24,
        Op::TailCall(_) => 25,
        Op::MakeArray { .. } => 26,
        Op::LoadIndex => 27,
        Op::StoreIndex => 28,
        Op::CallForeign(_) => 29,
        Op::CallBuiltin { .. } => 30,
        Op::LoadIndexUnchecked => OP_KIND_LOAD_INDEX_UNCHECKED,
        // Note: StructLiteral/GetField/SetField fall through to the
        // catch-all index below since they share their semantics with
        // the match path. Keep them grouped at the tail of the table.
        Op::StructLiteral { .. } => OP_KIND_STRUCT_LITERAL,
        Op::GetField { .. } => OP_KIND_GET_FIELD,
        Op::SetField { .. } => OP_KIND_SET_FIELD,
    }
}

const OP_KIND_LOAD_INDEX_UNCHECKED: usize = 31;
const OP_KIND_STRUCT_LITERAL: usize = 32;
const OP_KIND_GET_FIELD: usize = 33;
const OP_KIND_SET_FIELD: usize = 34;
const HANDLER_TABLE_LEN: usize = 35;

/// The dispatch table. Each entry is a handler keyed by the index
/// returned from `op_to_index`. Built once at compile time.
static HANDLERS: [Handler; HANDLER_TABLE_LEN] = {
    let mut table: [Handler; HANDLER_TABLE_LEN] = [h_unreachable; HANDLER_TABLE_LEN];
    table[0] = h_const;
    table[1] = h_add;
    table[2] = h_sub;
    table[3] = h_mul;
    table[4] = h_div;
    table[5] = h_mod;
    table[6] = h_neg;
    table[7] = h_load_local;
    table[8] = h_store_local;
    table[9] = h_call;
    table[10] = h_return_from_call;
    table[11] = h_jump;
    table[12] = h_jump_if_false;
    table[13] = h_jump_if_true;
    table[14] = h_inc_local;
    table[15] = h_eq;
    table[16] = h_neq;
    table[17] = h_lt;
    table[18] = h_le;
    table[19] = h_gt;
    table[20] = h_ge;
    table[21] = h_not;
    table[22] = h_return;
    table[23] = h_make_closure;
    table[24] = h_load_upvalue;
    table[25] = h_tail_call;
    table[26] = h_make_array;
    table[27] = h_load_index;
    table[28] = h_store_index;
    table[29] = h_call_foreign;
    table[30] = h_call_builtin;
    table[OP_KIND_LOAD_INDEX_UNCHECKED] = h_load_index_unchecked;
    table[OP_KIND_STRUCT_LITERAL] = h_struct_literal;
    table[OP_KIND_GET_FIELD] = h_get_field;
    table[OP_KIND_SET_FIELD] = h_set_field;
    table
};

/// Direct-threaded entry point. Mirrors `run_inner` byte-for-byte but
/// dispatches via the handler table. The outer error-wrapping logic
/// in `run_with` is shared, so `last_pc` updates are identical.
fn run_direct(
    program: &Program,
    last_pc: &mut (usize, usize),
    overflow_mode: OverflowMode,
) -> Result<Value, VmError> {
    let mut state = VmState {
        program,
        stack: Vec::with_capacity(64),
        locals: Vec::new(),
        frames: Vec::with_capacity(16),
        overflow_mode,
    };
    state.frames.push(CallFrame {
        chunk_idx: usize::MAX,
        pc: 0,
        locals_base: 0,
    });

    loop {
        let frame_idx = state.frame_idx();
        let chunk = state.current_chunk();
        let pc = state.frames[frame_idx].pc;
        *last_pc = (state.frames[frame_idx].chunk_idx, pc + 1);

        if pc >= chunk.code.len() {
            // Implicit return — main halts, fn body returns Void.
            if state.frames.len() == 1 {
                return Ok(state.stack.pop().unwrap_or(Value::Void));
            }
            let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
            state.locals.truncate(popped.locals_base);
            state.stack.push(Value::Void);
            continue;
        }

        let op = chunk.code[pc];
        state.frames[frame_idx].pc += 1;

        let handler = HANDLERS[op_to_index(op)];
        match handler(&mut state, op)? {
            Step::Continue => continue,
            Step::Halt(v) => return Ok(v),
        }
    }
}

// -----------------------------------------------------------------------------
// Handlers — each owns one opcode. Marked `#[inline(never)]` to keep
// each handler a separate code-cache entry; the indirect call through
// the dispatch table benefits from each call site building its own
// branch-predictor history.
// -----------------------------------------------------------------------------

#[inline(never)]
fn h_unreachable(_state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    // Unreachable for any op produced by `op_to_index`. Surface a clean
    // error if a future opcode is added without updating the table.
    Err(VmError::Unsupported(match op {
        Op::Const(_) => "Const",
        Op::Add => "Add",
        _ => "<unmapped op>",
    }))
}

#[inline(never)]
fn h_const(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::Const(idx) = op else { unreachable!() };
    let chunk = state.current_chunk();
    let v = chunk
        .constants
        .get(idx as usize)
        .ok_or(VmError::ConstantOutOfBounds(idx))?
        .clone();
    state.stack.push(v);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_add(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Add")?;
    state
        .stack
        .push(Value::Int(state.overflow_mode.add(a, b, "Add")?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_sub(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Sub")?;
    state
        .stack
        .push(Value::Int(state.overflow_mode.sub(a, b, "Sub")?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_mul(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Mul")?;
    state
        .stack
        .push(Value::Int(state.overflow_mode.mul(a, b, "Mul")?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_div(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Div")?;
    if b == 0 {
        return Err(VmError::DivideByZero);
    }
    state.stack.push(Value::Int(a / b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_mod(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Mod")?;
    if b == 0 {
        return Err(VmError::DivideByZero);
    }
    state.stack.push(Value::Int(a % b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_neg(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Int(i) = v else {
        return Err(VmError::TypeMismatch("Neg"));
    };
    state
        .stack
        .push(Value::Int(state.overflow_mode.neg(i, "Neg")?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_load_local(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::LoadLocal(idx) = op else {
        unreachable!()
    };
    let frame_idx = state.frame_idx();
    let base = state.frames[frame_idx].locals_base;
    let abs = base + idx as usize;
    let v = state
        .locals
        .get(abs)
        .ok_or(VmError::LocalOutOfBounds(idx))?
        .clone();
    state.stack.push(v);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_store_local(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::StoreLocal(idx) = op else {
        unreachable!()
    };
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let frame_idx = state.frame_idx();
    let base = state.frames[frame_idx].locals_base;
    let abs = base + idx as usize;
    if state.locals.len() <= abs {
        state.locals.resize(abs + 1, Value::Void);
    }
    state.locals[abs] = v;
    Ok(Step::Continue)
}

#[inline(never)]
fn h_call(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::Call(idx) = op else { unreachable!() };
    let func = state
        .program
        .functions
        .get(idx as usize)
        .ok_or(VmError::FunctionOutOfBounds(idx))?;
    let arity = func.arity as usize;
    if state.stack.len() < arity {
        return Err(VmError::EmptyStack);
    }
    if state.frames.len() >= MAX_CALL_DEPTH {
        return Err(VmError::CallStackOverflow);
    }
    let base = state.locals.len();
    state
        .locals
        .resize(base + func.local_count as usize, Value::Void);
    for i in (0..arity).rev() {
        let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
        state.locals[base + i] = v;
    }
    state.frames.push(CallFrame {
        chunk_idx: idx as usize,
        pc: 0,
        locals_base: base,
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_return_from_call(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let ret = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
    if state.frames.is_empty() {
        return Ok(Step::Halt(ret));
    }
    state.locals.truncate(popped.locals_base);
    state.stack.push(ret);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_jump(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::Jump(offset) = op else { unreachable!() };
    let frame_idx = state.frame_idx();
    let chunk_len = state.current_chunk().code.len();
    let new_pc = (state.frames[frame_idx].pc as isize) + offset as isize;
    if new_pc < 0 || (new_pc as usize) > chunk_len {
        return Err(VmError::JumpOutOfBounds);
    }
    state.frames[frame_idx].pc = new_pc as usize;
    Ok(Step::Continue)
}

#[inline(never)]
fn h_jump_if_false(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::JumpIfFalse(offset) = op else {
        unreachable!()
    };
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let is_falsy = match v {
        Value::Bool(b) => !b,
        Value::Int(i) => i == 0,
        _ => return Err(VmError::TypeMismatch("JumpIfFalse")),
    };
    if is_falsy {
        let frame_idx = state.frame_idx();
        let chunk_len = state.current_chunk().code.len();
        let new_pc = (state.frames[frame_idx].pc as isize) + offset as isize;
        if new_pc < 0 || (new_pc as usize) > chunk_len {
            return Err(VmError::JumpOutOfBounds);
        }
        state.frames[frame_idx].pc = new_pc as usize;
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_jump_if_true(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::JumpIfTrue(offset) = op else {
        unreachable!()
    };
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let is_truthy = match v {
        Value::Bool(b) => b,
        Value::Int(i) => i != 0,
        _ => return Err(VmError::TypeMismatch("JumpIfTrue")),
    };
    if is_truthy {
        let frame_idx = state.frame_idx();
        let chunk_len = state.current_chunk().code.len();
        let new_pc = (state.frames[frame_idx].pc as isize) + offset as isize;
        if new_pc < 0 || (new_pc as usize) > chunk_len {
            return Err(VmError::JumpOutOfBounds);
        }
        state.frames[frame_idx].pc = new_pc as usize;
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_inc_local(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::IncLocal(idx) = op else {
        unreachable!()
    };
    let frame_idx = state.frame_idx();
    let base = state.frames[frame_idx].locals_base;
    let abs = base + idx as usize;
    let v = state
        .locals
        .get(abs)
        .ok_or(VmError::LocalOutOfBounds(idx))?;
    let Value::Int(n) = *v else {
        return Err(VmError::TypeMismatch("IncLocal"));
    };
    state.locals[abs] = Value::Int(state.overflow_mode.add(n, 1, "IncLocal")?);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_eq(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Eq")?;
    state.stack.push(Value::Bool(a == b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_neq(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Neq")?;
    state.stack.push(Value::Bool(a != b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_lt(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Lt")?;
    state.stack.push(Value::Bool(a < b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_le(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Le")?;
    state.stack.push(Value::Bool(a <= b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_gt(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Gt")?;
    state.stack.push(Value::Bool(a > b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_ge(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Ge")?;
    state.stack.push(Value::Bool(a >= b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_not(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Bool(b) = v else {
        return Err(VmError::TypeMismatch("Not"));
    };
    state.stack.push(Value::Bool(!b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_return(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Ok(Step::Halt(state.stack.pop().unwrap_or(Value::Void)))
}

#[inline(never)]
fn h_make_closure(_state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Err(VmError::Unsupported("MakeClosure"))
}

#[inline(never)]
fn h_load_upvalue(_state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Err(VmError::Unsupported("LoadUpvalue"))
}

#[inline(never)]
fn h_tail_call(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::TailCall(idx) = op else {
        unreachable!()
    };
    let func = state
        .program
        .functions
        .get(idx as usize)
        .ok_or(VmError::FunctionOutOfBounds(idx))?;
    let arity = func.arity as usize;
    if state.stack.len() < arity {
        return Err(VmError::EmptyStack);
    }
    let frame_idx = state.frame_idx();
    let base = state.frames[frame_idx].locals_base;
    for i in (0..arity).rev() {
        let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
        state.locals[base + i] = v;
    }
    state.frames[frame_idx].pc = 0;
    Ok(Step::Continue)
}

#[inline(never)]
fn h_make_array(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::MakeArray { len } = op else {
        unreachable!()
    };
    let n = len as usize;
    if state.stack.len() < n {
        return Err(VmError::EmptyStack);
    }
    let split_at = state.stack.len() - n;
    let items: Vec<Value> = state.stack.drain(split_at..).collect();
    state.stack.push(Value::Array(items));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_load_index(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let idx_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let arr_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Int(idx) = idx_val else {
        return Err(VmError::TypeMismatch("LoadIndex (non-int index)"));
    };
    let Value::Array(items) = arr_val else {
        return Err(VmError::TypeMismatch("LoadIndex (non-array target)"));
    };
    if idx < 0 || (idx as usize) >= items.len() {
        return Err(VmError::ArrayIndexOutOfBounds {
            index: idx,
            len: items.len(),
        });
    }
    state.stack.push(items[idx as usize].clone());
    Ok(Step::Continue)
}

/// RES-407: bounds-check-elided sibling of [`h_load_index`]. Type
/// checks on operands are kept — the verifier rules out a stale
/// in-range claim only on the index value, not on type confusion.
#[inline(never)]
fn h_load_index_unchecked(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let idx_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let arr_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Int(idx) = idx_val else {
        return Err(VmError::TypeMismatch("LoadIndexUnchecked (non-int index)"));
    };
    let Value::Array(items) = arr_val else {
        return Err(VmError::TypeMismatch(
            "LoadIndexUnchecked (non-array target)",
        ));
    };
    state.stack.push(items[idx as usize].clone());
    Ok(Step::Continue)
}

#[inline(never)]
fn h_store_index(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let idx_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let arr_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Int(idx) = idx_val else {
        return Err(VmError::TypeMismatch("StoreIndex (non-int index)"));
    };
    let Value::Array(mut items) = arr_val else {
        return Err(VmError::TypeMismatch("StoreIndex (non-array target)"));
    };
    if idx < 0 || (idx as usize) >= items.len() {
        return Err(VmError::ArrayIndexOutOfBounds {
            index: idx,
            len: items.len(),
        });
    }
    items[idx as usize] = v;
    state.stack.push(Value::Array(items));
    Ok(Step::Continue)
}

#[cfg(feature = "ffi")]
#[inline(never)]
fn h_call_foreign(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::CallForeign(idx) = op else {
        unreachable!()
    };
    let sym = state
        .program
        .foreign_syms
        .get(idx as usize)
        .ok_or(VmError::FunctionOutOfBounds(idx))?;
    let arity = sym.sig.params.len();
    if state.stack.len() < arity {
        return Err(VmError::EmptyStack);
    }
    let mut args: Vec<crate::Value> = (0..arity)
        .map(|_| state.stack.pop().expect("checked above"))
        .collect();
    args.reverse();
    let result =
        crate::ffi_trampolines::call_foreign(sym, &args).map_err(VmError::ForeignCallFailed)?;
    state.stack.push(result);
    Ok(Step::Continue)
}

#[cfg(not(feature = "ffi"))]
#[inline(never)]
fn h_call_foreign(_state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Err(VmError::Unsupported(
        "CallForeign (build without --features ffi)",
    ))
}

#[inline(never)]
fn h_call_builtin(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::CallBuiltin { name_const, arity } = op else {
        unreachable!()
    };
    let chunk = state.current_chunk();
    let name_val = chunk
        .constants
        .get(name_const as usize)
        .ok_or(VmError::ConstantOutOfBounds(name_const))?;
    let name = match name_val {
        Value::String(s) => s.clone(),
        _ => return Err(VmError::TypeMismatch("CallBuiltin (non-string name)")),
    };
    let func = crate::lookup_builtin(&name).ok_or_else(|| VmError::UnknownBuiltin(name.clone()))?;
    let n = arity as usize;
    if state.stack.len() < n {
        return Err(VmError::EmptyStack);
    }
    let mut args: Vec<Value> = (0..n)
        .map(|_| state.stack.pop().expect("checked above"))
        .collect();
    args.reverse();
    let result = func(&args).map_err(VmError::BuiltinCallFailed)?;
    state.stack.push(result);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_struct_literal(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::StructLiteral {
        name_const,
        field_count,
    } = op
    else {
        unreachable!()
    };
    let chunk = state.current_chunk();
    let name = constant_as_string(chunk, name_const, "StructLiteral (type name)")?;
    let n = field_count as usize;
    let needed = n.checked_mul(2).ok_or(VmError::EmptyStack)?;
    if state.stack.len() < needed {
        return Err(VmError::EmptyStack);
    }
    let split_at = state.stack.len() - needed;
    let flat: Vec<Value> = state.stack.drain(split_at..).collect();
    let mut fields: Vec<(String, Value)> = Vec::with_capacity(n);
    let mut it = flat.into_iter();
    for _ in 0..n {
        let k = it.next().ok_or(VmError::EmptyStack)?;
        let v = it.next().ok_or(VmError::EmptyStack)?;
        let Value::String(field_name) = k else {
            return Err(VmError::TypeMismatch("StructLiteral (non-string key)"));
        };
        fields.push((field_name, v));
    }
    state.stack.push(Value::Struct { name, fields });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_get_field(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::GetField { name_const } = op else {
        unreachable!()
    };
    let chunk = state.current_chunk();
    let field = constant_as_string(chunk, name_const, "GetField (field name)")?;
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Struct {
        name: sname,
        fields,
    } = v
    else {
        return Err(VmError::TypeMismatch("GetField (non-struct target)"));
    };
    let found = fields
        .iter()
        .find(|(k, _)| k == &field)
        .map(|(_, v)| v.clone());
    match found {
        Some(val) => {
            state.stack.push(val);
            Ok(Step::Continue)
        }
        None => Err(VmError::UnknownField {
            struct_name: sname,
            field,
        }),
    }
}

#[inline(never)]
fn h_set_field(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::SetField { name_const } = op else {
        unreachable!()
    };
    let chunk = state.current_chunk();
    let field = constant_as_string(chunk, name_const, "SetField (field name)")?;
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let tgt = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Struct {
        name: sname,
        mut fields,
    } = tgt
    else {
        return Err(VmError::TypeMismatch("SetField (non-struct target)"));
    };
    let slot = fields.iter_mut().find(|(k, _)| k == &field);
    match slot {
        Some((_, existing)) => *existing = v,
        None => {
            return Err(VmError::UnknownField {
                struct_name: sname,
                field,
            });
        }
    }
    state.stack.push(Value::Struct {
        name: sname,
        fields,
    });
    Ok(Step::Continue)
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
        Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        }
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
        assert!(
            display.contains("divide by zero"),
            "missing kind: {}",
            display
        );
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
        let p = Program {
            main,
            functions: vec![runaway],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
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
        let src =
            "fn fib(int n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fib(10);";
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
    fn for_in_compiles_and_runs() {
        // RES-334: `for x in arr { ... }` now compiles directly to
        // bytecode (was scoped out by RES-083; the original assertion
        // was inverted here when this ticket landed). Sums [1, 2, 3]
        // into `total` and returns it.
        let result = compile_run(
            "let xs = [1, 2, 3]; let total = 0; for x in xs { total = total + x; } total;",
        )
        .expect("for-in must compile and run");
        assert_int(result, 6);
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

    // ---------- RES-384: tail-call optimisation ----------

    #[test]
    fn res384_tail_recursive_sum_does_not_overflow() {
        // Without TCO this would exceed MAX_CALL_DEPTH (1024) at n=100_000.
        // With TCO the call stack stays at depth 1.
        // sum(0, 100) = 1+2+...+100 = 5050
        let src = "fn sum(int acc, int n) { if n == 0 { return acc; } return sum(acc + n, n - 1); } sum(0, 100);";
        assert_int(compile_run(src).unwrap(), 5050);
    }

    #[test]
    fn res384_tail_recursive_sum_large_does_not_overflow() {
        // 100_000 iterations — would blow the 1024-frame cap without TCO.
        let src = r#"
            fn sum(int acc, int n) {
                if n == 0 { return acc; }
                return sum(acc + n, n - 1);
            }
            sum(0, 100000);
        "#;
        // sum(0, 100_000) = 100_000 * 100_001 / 2 = 5_000_050_000
        let result = compile_run(src).unwrap();
        assert_int(result, 5_000_050_000i64);
    }

    #[test]
    fn res384_tail_call_opcode_is_emitted_for_self_tail_call() {
        // After compilation the function body should contain TailCall,
        // not a plain Call followed by ReturnFromCall.
        let src = "fn count(int n) { if n == 0 { return 0; } return count(n - 1); } count(5);";
        let (ast, _) = crate::parse(src);
        let prog = crate::compiler::compile(&ast).unwrap();
        let f = &prog.functions[0];
        let has_tail_call = f
            .chunk
            .code
            .iter()
            .any(|op| matches!(op, crate::bytecode::Op::TailCall(0)));
        assert!(
            has_tail_call,
            "expected TailCall(0) in fn body: {:?}",
            f.chunk.code
        );
    }

    #[test]
    fn res384_non_self_call_still_uses_regular_call() {
        // A call to a *different* function must not be promoted to TailCall.
        let src =
            "fn double(int n) { return n + n; } fn wrap(int n) { return double(n); } wrap(3);";
        let (ast, _) = crate::parse(src);
        let prog = crate::compiler::compile(&ast).unwrap();
        // fn wrap (index 1) calls fn double (index 0) — must remain Call(0)
        let wrap = &prog.functions[1];
        let has_regular_call = wrap
            .chunk
            .code
            .iter()
            .any(|op| matches!(op, crate::bytecode::Op::Call(0)));
        assert!(
            has_regular_call,
            "cross-function call should remain Call, not TailCall: {:?}",
            wrap.chunk.code
        );
    }

    #[test]
    fn res384_mutual_recursion_still_works_via_call() {
        // Mutual recursion (is_even/is_odd) cannot use TailCall (different
        // functions) — must still run correctly via regular Call.
        let src = r#"
            fn is_even(int n) { if n == 0 { return 1; } return is_odd(n - 1); }
            fn is_odd(int n) { if n == 0 { return 0; } return is_even(n - 1); }
            is_even(10);
        "#;
        assert_int(compile_run(src).unwrap(), 1); // 10 is even → 1
    }

    #[test]
    fn res384_non_tail_recursive_still_works() {
        // A non-tail-recursive function (fib) must still produce correct
        // results — the rewriting pass must only touch self-tail-calls.
        let src =
            "fn fib(int n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fib(10);";
        assert_int(compile_run(src).unwrap(), 55);
    }

    // ---------- RES-169a: skeleton closure-opcode dispatch ----------

    #[test]
    fn res169a_make_closure_dispatch_returns_unsupported() {
        // MakeClosure is laid down as a skeleton in RES-169a; the
        // compiler never emits it yet, so if it shows up in a
        // chunk that's a wiring bug. The dispatch arm reports that
        // cleanly via `VmError::Unsupported("MakeClosure")`.
        let p = const_program(
            &[],
            &[Op::MakeClosure {
                fn_idx: 0,
                upvalue_count: 0,
            }],
        );
        let err = run(&p).unwrap_err();
        match err.kind() {
            VmError::Unsupported(what) => assert_eq!(*what, "MakeClosure"),
            other => panic!("expected Unsupported(MakeClosure), got {:?}", other),
        }
    }

    #[test]
    fn res169a_load_upvalue_dispatch_returns_unsupported() {
        let p = const_program(&[], &[Op::LoadUpvalue(0)]);
        let err = run(&p).unwrap_err();
        match err.kind() {
            VmError::Unsupported(what) => assert_eq!(*what, "LoadUpvalue"),
            other => panic!("expected Unsupported(LoadUpvalue), got {:?}", other),
        }
    }

    #[test]
    fn res169a_unsupported_error_display_is_descriptive() {
        let e = VmError::Unsupported("MakeClosure");
        assert_eq!(e.to_string(), "vm: unsupported opcode: MakeClosure");
    }

    #[test]
    fn res169a_closure_value_variant_constructs() {
        // `Value::Closure` is a skeleton variant — not constructed
        // by the interpreter or VM today, but must be usable when
        // RES-169c hooks up the dispatch. Build one directly and
        // sanity-check its Debug + Display outputs.
        let c = Value::Closure {
            fn_idx: 5,
            upvalues: vec![Value::Int(1), Value::Int(2)].into_boxed_slice(),
        };
        assert_eq!(format!("{:?}", c), "Closure(fn=5, 2 upvalues)");
        assert_eq!(format!("{}", c), "<closure>");
    }

    #[test]
    fn res169a_existing_vm_path_still_returns_correct_result() {
        // Regression guard: adding the new Value / Op / VmError
        // variants must not regress any existing opcode's
        // behaviour. Smoke-test a non-trivial arithmetic program.
        let p = const_program(
            &[Value::Int(10), Value::Int(32)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        assert_int(run(&p).unwrap(), 42);
    }

    // ---------- RES-171a: array ops ----------

    fn assert_int_array(actual: Value, expected: &[i64]) {
        match actual {
            Value::Array(items) => {
                let got: Vec<i64> = items
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => *n,
                        other => panic!("expected Int in array, got {:?}", other),
                    })
                    .collect();
                assert_eq!(got, expected, "array contents mismatch");
            }
            other => panic!("expected Array({:?}), got {:?}", expected, other),
        }
    }

    #[test]
    fn res171a_make_array_from_three_constants() {
        // Push 1, 2, 3 and wrap them with MakeArray(3).
        let p = const_program(
            &[Value::Int(1), Value::Int(2), Value::Int(3)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::MakeArray { len: 3 },
                Op::Return,
            ],
        );
        assert_int_array(run(&p).unwrap(), &[1, 2, 3]);
    }

    #[test]
    fn res171a_make_array_empty_literal_returns_empty_array() {
        let p = const_program(&[], &[Op::MakeArray { len: 0 }, Op::Return]);
        assert_int_array(run(&p).unwrap(), &[]);
    }

    #[test]
    fn res171a_make_array_stack_underflow_errors() {
        // Only one item on the stack but MakeArray asks for three.
        let p = const_program(
            &[Value::Int(1)],
            &[Op::Const(0), Op::MakeArray { len: 3 }, Op::Return],
        );
        let err = run(&p).unwrap_err();
        assert!(matches!(err.kind(), VmError::EmptyStack), "{:?}", err);
    }

    #[test]
    fn res171a_load_index_reads_element() {
        // [10, 20, 30][1] == 20
        let p = const_program(
            &[
                Value::Int(10),
                Value::Int(20),
                Value::Int(30),
                Value::Int(1),
            ],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::MakeArray { len: 3 },
                Op::Const(3),
                Op::LoadIndex,
                Op::Return,
            ],
        );
        assert_int(run(&p).unwrap(), 20);
    }

    #[test]
    fn res171a_load_index_out_of_bounds_errors() {
        let p = const_program(
            &[Value::Int(1), Value::Int(2), Value::Int(5)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::MakeArray { len: 2 },
                Op::Const(2),
                Op::LoadIndex,
                Op::Return,
            ],
        );
        let err = run(&p).unwrap_err();
        match err.kind() {
            VmError::ArrayIndexOutOfBounds { index, len } => {
                assert_eq!(*index, 5);
                assert_eq!(*len, 2);
            }
            other => panic!("expected OOB, got {:?}", other),
        }
    }

    #[test]
    fn res171a_load_index_negative_index_errors() {
        let p = const_program(
            &[Value::Int(1), Value::Int(2), Value::Int(-1)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::MakeArray { len: 2 },
                Op::Const(2),
                Op::LoadIndex,
                Op::Return,
            ],
        );
        let err = run(&p).unwrap_err();
        assert!(matches!(
            err.kind(),
            VmError::ArrayIndexOutOfBounds { index: -1, len: 2 }
        ));
    }

    #[test]
    fn res171a_load_index_non_int_errors() {
        // Index is a Bool — type mismatch.
        let p = const_program(
            &[Value::Int(1), Value::Bool(true)],
            &[
                Op::Const(0),
                Op::MakeArray { len: 1 },
                Op::Const(1),
                Op::LoadIndex,
                Op::Return,
            ],
        );
        let err = run(&p).unwrap_err();
        assert!(matches!(err.kind(), VmError::TypeMismatch(_)));
    }

    #[test]
    fn res171a_store_index_writes_and_pushes_modified_array() {
        // Build [1, 2, 3], then write 99 to index 1, return the array.
        let p = const_program(
            &[
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(1),
                Value::Int(99),
            ],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::MakeArray { len: 3 },
                Op::Const(3),
                Op::Const(4),
                Op::StoreIndex,
                Op::Return,
            ],
        );
        assert_int_array(run(&p).unwrap(), &[1, 99, 3]);
    }

    #[test]
    fn res171a_store_index_oob_errors_without_modifying() {
        let p = const_program(
            &[Value::Int(1), Value::Int(2), Value::Int(5), Value::Int(99)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::MakeArray { len: 2 },
                Op::Const(2),
                Op::Const(3),
                Op::StoreIndex,
                Op::Return,
            ],
        );
        let err = run(&p).unwrap_err();
        assert!(matches!(
            err.kind(),
            VmError::ArrayIndexOutOfBounds { index: 5, len: 2 }
        ));
    }

    #[test]
    fn res171a_store_index_display_is_descriptive() {
        let e = VmError::ArrayIndexOutOfBounds { index: 7, len: 3 };
        assert_eq!(
            e.to_string(),
            "vm: array index 7 out of bounds for length 3"
        );
    }

    // ---- RES-171a: compile + run roundtrips (integration) ----

    #[test]
    fn res171a_compile_and_run_array_literal_index() {
        // let a = [10, 20, 30]; return a[1];  => 20
        let v = compile_run("let a = [10, 20, 30]; return a[1];").unwrap();
        assert_int(v, 20);
    }

    #[test]
    fn res171a_compile_and_run_index_assign_then_read() {
        // let a = [1,2,3]; a[1] = 99; return a[1];  => 99
        let v = compile_run("let a = [1, 2, 3]; a[1] = 99; return a[1];").unwrap();
        assert_int(v, 99);
    }

    #[test]
    fn res171a_compile_and_run_read_all_after_store() {
        // Store preserves the other elements.
        let v = compile_run("let a = [10, 20, 30]; a[0] = 100; return a[2];").unwrap();
        assert_int(v, 30);
    }

    #[test]
    fn res171a_compile_rejects_nested_index_assignment() {
        // a[i][j] = v is RES-171c — today we emit a clean error.
        let (program, _) = crate::parse("let a = [[1,2],[3,4]]; a[0][1] = 99; return 0;");
        let err = crate::compiler::compile(&program).unwrap_err();
        match err {
            crate::bytecode::CompileError::Unsupported(msg) => {
                assert!(msg.contains("nested"), "unexpected msg: {}", msg);
            }
            other => panic!("expected Unsupported, got {:?}", other),
        }
    }

    #[test]
    fn res171a_empty_array_literal_compiles_and_runs() {
        // An empty array literal produces MakeArray { len: 0 }.
        let v = compile_run("let a = []; return 0;").unwrap();
        assert_int(v, 0);
    }

    #[test]
    fn res171a_oob_read_from_compiled_program_surfaces_at_line() {
        // Runtime OOB from a compiled program should come through
        // the AtLine wrapper so the user sees a line number.
        let (program, _) = crate::parse("let a = [1, 2]; return a[5];");
        let prog = crate::compiler::compile(&program).unwrap();
        let err = run(&prog).unwrap_err();
        // Line wrapper carries the kind for us.
        assert!(matches!(
            err.kind(),
            VmError::ArrayIndexOutOfBounds { index: 5, len: 2 }
        ));
    }

    // ============================================================
    // RES-335: struct opcodes
    // ============================================================

    #[test]
    fn res335_struct_literal_constructs_value() {
        // Synthetic chunk: build `Point { x: 1, y: 2 }` and return it.
        let mut main = Chunk::new();
        let type_const = main.add_constant(Value::String("Point".into())).unwrap();
        let x_const = main.add_constant(Value::String("x".into())).unwrap();
        let y_const = main.add_constant(Value::String("y".into())).unwrap();
        let one = main.add_constant(Value::Int(1)).unwrap();
        let two = main.add_constant(Value::Int(2)).unwrap();
        main.emit(Op::Const(x_const), 1);
        main.emit(Op::Const(one), 1);
        main.emit(Op::Const(y_const), 1);
        main.emit(Op::Const(two), 1);
        main.emit(
            Op::StructLiteral {
                name_const: type_const,
                field_count: 2,
            },
            1,
        );
        main.emit(Op::Return, 1);
        let prog = Program {
            main,
            functions: vec![],
            #[cfg(feature = "ffi")]
            foreign_syms: vec![],
        };
        let result = run(&prog).unwrap();
        match result {
            Value::Struct { name, fields } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].0, "x");
                assert!(matches!(fields[0].1, Value::Int(1)));
                assert_eq!(fields[1].0, "y");
                assert!(matches!(fields[1].1, Value::Int(2)));
            }
            other => panic!("expected Struct, got {:?}", other),
        }
    }

    #[test]
    fn res335_get_field_pushes_field_value() {
        assert_int(
            compile_run(
                "struct Point { int x, int y, } \
                 fn main() -> int { let p = new Point { x: 1, y: 42 }; return p.y; } \
                 main();",
            )
            .unwrap(),
            42,
        );
    }

    #[test]
    fn res335_set_field_updates_in_place() {
        assert_int(
            compile_run(
                "struct Point { int x, int y, } \
                 fn main() -> int { \
                     let p = new Point { x: 1, y: 2 }; \
                     p.x = 10; \
                     p.y = 20; \
                     return p.x + p.y; \
                 } \
                 main();",
            )
            .unwrap(),
            30,
        );
    }

    #[test]
    fn res335_get_field_on_non_struct_errors() {
        // Build chunk that pushes an int then GetField — type mismatch.
        let mut main = Chunk::new();
        let f_const = main.add_constant(Value::String("x".into())).unwrap();
        let v_const = main.add_constant(Value::Int(5)).unwrap();
        main.emit(Op::Const(v_const), 1);
        main.emit(
            Op::GetField {
                name_const: f_const,
            },
            1,
        );
        main.emit(Op::Return, 1);
        let prog = Program {
            main,
            functions: vec![],
            #[cfg(feature = "ffi")]
            foreign_syms: vec![],
        };
        let err = run(&prog).unwrap_err();
        assert!(
            matches!(err.kind(), VmError::TypeMismatch(_)),
            "got {:?}",
            err
        );
    }

    #[test]
    fn res335_get_field_unknown_field_errors() {
        let mut main = Chunk::new();
        let type_const = main.add_constant(Value::String("Point".into())).unwrap();
        let x_const = main.add_constant(Value::String("x".into())).unwrap();
        let one = main.add_constant(Value::Int(1)).unwrap();
        let missing = main.add_constant(Value::String("z".into())).unwrap();
        main.emit(Op::Const(x_const), 1);
        main.emit(Op::Const(one), 1);
        main.emit(
            Op::StructLiteral {
                name_const: type_const,
                field_count: 1,
            },
            1,
        );
        main.emit(
            Op::GetField {
                name_const: missing,
            },
            1,
        );
        main.emit(Op::Return, 1);
        let prog = Program {
            main,
            functions: vec![],
            #[cfg(feature = "ffi")]
            foreign_syms: vec![],
        };
        let err = run(&prog).unwrap_err();
        match err.kind() {
            VmError::UnknownField { struct_name, field } => {
                assert_eq!(struct_name, "Point");
                assert_eq!(field, "z");
            }
            other => panic!("expected UnknownField, got {:?}", other),
        }
    }

    #[test]
    fn res335_sizeof_op_stays_bounded() {
        // The bytecode module claims sizeof::<Op>() <= 8. RES-335 added
        // variants carrying `u16 + u16` payloads; verify the envelope
        // didn't inflate.
        assert!(
            std::mem::size_of::<Op>() <= 8,
            "sizeof(Op) = {} bytes; struct opcodes should not inflate this",
            std::mem::size_of::<Op>()
        );
    }

    // ---------------------------------------------------------------
    // RES-349: overflow-mode tests. Each builds a tiny program that
    // pushes two i64 constants and runs Op::Add / Op::Sub / Op::Mul.
    // We use `run_with_mode` so the tests are independent of process
    // env (the env-var path is exercised separately below).
    // ---------------------------------------------------------------

    fn add_program(a: i64, b: i64) -> Program {
        const_program(
            &[Value::Int(a), Value::Int(b)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        )
    }

    fn sub_program(a: i64, b: i64) -> Program {
        const_program(
            &[Value::Int(a), Value::Int(b)],
            &[Op::Const(0), Op::Const(1), Op::Sub, Op::Return],
        )
    }

    fn mul_program(a: i64, b: i64) -> Program {
        const_program(
            &[Value::Int(a), Value::Int(b)],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        )
    }

    fn neg_program(a: i64) -> Program {
        const_program(&[Value::Int(a)], &[Op::Const(0), Op::Neg, Op::Return])
    }

    #[test]
    fn res349_default_mode_is_wrap() {
        // Default mode (no env var): adding 1 to i64::MAX wraps to MIN.
        // This pins the byte-identical pre-RES-349 behaviour.
        let prog = add_program(i64::MAX, 1);
        assert_int(run_with_mode(&prog, OverflowMode::Wrap).unwrap(), i64::MIN);
    }

    #[test]
    fn res349_add_wraps_under_wrap_mode() {
        let prog = add_program(i64::MAX, 1);
        assert_int(run_with_mode(&prog, OverflowMode::Wrap).unwrap(), i64::MIN);
    }

    #[test]
    fn res349_add_saturates_under_saturate_mode() {
        let prog = add_program(i64::MAX, 1);
        assert_int(
            run_with_mode(&prog, OverflowMode::Saturate).unwrap(),
            i64::MAX,
        );
    }

    #[test]
    fn res349_add_traps_under_trap_mode() {
        let prog = add_program(i64::MAX, 1);
        let err = run_with_mode(&prog, OverflowMode::Trap).unwrap_err();
        assert_eq!(err.kind(), &VmError::IntegerOverflow("Add"));
    }

    #[test]
    fn res349_sub_wraps_under_wrap_mode() {
        let prog = sub_program(i64::MIN, 1);
        assert_int(run_with_mode(&prog, OverflowMode::Wrap).unwrap(), i64::MAX);
    }

    #[test]
    fn res349_sub_saturates_under_saturate_mode() {
        let prog = sub_program(i64::MIN, 1);
        assert_int(
            run_with_mode(&prog, OverflowMode::Saturate).unwrap(),
            i64::MIN,
        );
    }

    #[test]
    fn res349_sub_traps_under_trap_mode() {
        let prog = sub_program(i64::MIN, 1);
        let err = run_with_mode(&prog, OverflowMode::Trap).unwrap_err();
        assert_eq!(err.kind(), &VmError::IntegerOverflow("Sub"));
    }

    #[test]
    fn res349_mul_wraps_under_wrap_mode() {
        let prog = mul_program(i64::MAX, 2);
        assert_int(
            run_with_mode(&prog, OverflowMode::Wrap).unwrap(),
            i64::MAX.wrapping_mul(2),
        );
    }

    #[test]
    fn res349_mul_saturates_under_saturate_mode() {
        let prog = mul_program(i64::MAX, 2);
        assert_int(
            run_with_mode(&prog, OverflowMode::Saturate).unwrap(),
            i64::MAX,
        );
    }

    #[test]
    fn res349_mul_traps_under_trap_mode() {
        let prog = mul_program(i64::MAX, 2);
        let err = run_with_mode(&prog, OverflowMode::Trap).unwrap_err();
        assert_eq!(err.kind(), &VmError::IntegerOverflow("Mul"));
    }

    #[test]
    fn res349_neg_traps_on_i64_min_under_trap_mode() {
        // -i64::MIN cannot be represented as i64 — Trap surfaces, Wrap
        // returns i64::MIN (the documented wrapping behaviour).
        let prog = neg_program(i64::MIN);
        let err = run_with_mode(&prog, OverflowMode::Trap).unwrap_err();
        assert_eq!(err.kind(), &VmError::IntegerOverflow("Neg"));
        assert_int(run_with_mode(&prog, OverflowMode::Wrap).unwrap(), i64::MIN);
        assert_int(
            run_with_mode(&prog, OverflowMode::Saturate).unwrap(),
            i64::MAX,
        );
    }

    #[test]
    fn res349_normal_arithmetic_unchanged_under_all_modes() {
        // Non-overflowing arithmetic must produce identical results
        // regardless of mode — only the overflow corner cases differ.
        for &mode in &[
            OverflowMode::Wrap,
            OverflowMode::Saturate,
            OverflowMode::Trap,
        ] {
            assert_int(run_with_mode(&add_program(2, 3), mode).unwrap(), 5);
            assert_int(run_with_mode(&sub_program(10, 4), mode).unwrap(), 6);
            assert_int(run_with_mode(&mul_program(6, 7), mode).unwrap(), 42);
        }
    }

    #[test]
    fn res349_overflow_mode_from_env_parses_known_values() {
        // Drive `from_env` directly without mutating process env in
        // parallel-test-friendly ways: round-trip the parser via a
        // serial section.
        let saved = std::env::var("RESILIENT_OVERFLOW_MODE").ok();
        // wrap (default)
        unsafe {
            std::env::remove_var("RESILIENT_OVERFLOW_MODE");
        }
        assert_eq!(OverflowMode::from_env(), OverflowMode::Wrap);
        unsafe {
            std::env::set_var("RESILIENT_OVERFLOW_MODE", "saturate");
        }
        assert_eq!(OverflowMode::from_env(), OverflowMode::Saturate);
        unsafe {
            std::env::set_var("RESILIENT_OVERFLOW_MODE", "trap");
        }
        assert_eq!(OverflowMode::from_env(), OverflowMode::Trap);
        // Unknown values fall back to Wrap.
        unsafe {
            std::env::set_var("RESILIENT_OVERFLOW_MODE", "bogus");
        }
        assert_eq!(OverflowMode::from_env(), OverflowMode::Wrap);
        // Restore.
        unsafe {
            match saved {
                Some(v) => std::env::set_var("RESILIENT_OVERFLOW_MODE", v),
                None => std::env::remove_var("RESILIENT_OVERFLOW_MODE"),
            }
        }
    }

    #[cfg(feature = "ffi")]
    #[test]
    fn vm_calls_foreign_via_call_foreign_opcode() {
        use crate::bytecode::{Chunk, Op, Program};
        use crate::ffi::{FfiType, ForeignSignature, ForeignSymbol};

        extern "C" fn double_it(x: i64) -> i64 {
            x * 2
        }

        let sig = ForeignSignature {
            params: vec![FfiType::Int],
            ret: FfiType::Int,
        };
        let sym = std::sync::Arc::new(ForeignSymbol {
            name: "double_it".into(),
            ptr: double_it as *const (),
            sig,
        });

        // Build a tiny program: push 21, CallForeign(0), Return.
        let mut main = Chunk::new();
        main.constants.push(crate::Value::Int(21));
        main.emit(Op::Const(0), 1);
        main.emit(Op::CallForeign(0), 1);
        main.emit(Op::Return, 1);

        let prog = Program {
            main,
            functions: vec![],
            foreign_syms: vec![sym],
        };

        let result = run(&prog).expect("vm run");
        assert!(matches!(result, crate::Value::Int(42)), "got {:?}", result);
    }

    // ============================================================
    // RES-329: direct-threaded dispatch — equivalence tests
    // ============================================================
    //
    // Each program below runs once under `Dispatch::Match` (the default
    // pre-RES-329 path) and once under `Dispatch::Direct` (the new
    // function-pointer table). The two results must match byte-for-byte.

    fn run_both(src: &str) -> (Result<Value, VmError>, Result<Value, VmError>) {
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        let m = run_with(&prog, OverflowMode::Wrap, Dispatch::Match);
        let d = run_with(&prog, OverflowMode::Wrap, Dispatch::Direct);
        (m, d)
    }

    fn value_repr(v: &Value) -> String {
        // Value lacks PartialEq; Debug is total and stable, so compare
        // its formatted output to verify the two paths agree.
        format!("{:?}", v)
    }

    fn assert_both_eq(src: &str) {
        let (m, d) = run_both(src);
        match (&m, &d) {
            (Ok(a), Ok(b)) => assert_eq!(
                value_repr(a),
                value_repr(b),
                "match vs direct disagree on {:?}",
                src
            ),
            (Err(a), Err(b)) => assert_eq!(
                a.kind(),
                b.kind(),
                "match vs direct disagree on error for {:?}",
                src
            ),
            _ => panic!(
                "match vs direct disagree on outcome shape for {:?}: match={:?}, direct={:?}",
                src, m, d
            ),
        }
    }

    #[test]
    fn res329_dispatch_from_env_defaults_to_match() {
        let saved = std::env::var("RESILIENT_DISPATCH").ok();
        unsafe {
            std::env::remove_var("RESILIENT_DISPATCH");
        }
        assert_eq!(Dispatch::from_env(), Dispatch::Match);
        unsafe {
            std::env::set_var("RESILIENT_DISPATCH", "direct");
        }
        assert_eq!(Dispatch::from_env(), Dispatch::Direct);
        unsafe {
            std::env::set_var("RESILIENT_DISPATCH", "DIRECT");
        }
        assert_eq!(Dispatch::from_env(), Dispatch::Direct);
        unsafe {
            std::env::set_var("RESILIENT_DISPATCH", "threaded");
        }
        assert_eq!(Dispatch::from_env(), Dispatch::Direct);
        unsafe {
            std::env::set_var("RESILIENT_DISPATCH", "garbage");
        }
        assert_eq!(Dispatch::from_env(), Dispatch::Match);
        unsafe {
            match saved {
                Some(v) => std::env::set_var("RESILIENT_DISPATCH", v),
                None => std::env::remove_var("RESILIENT_DISPATCH"),
            }
        }
    }

    #[test]
    fn res329_direct_arithmetic_matches_match() {
        assert_both_eq("2 + 3 * 4;");
        assert_both_eq("let x = 9; x;");
        assert_both_eq("100 - 50 / 5 + 7;");
        assert_both_eq("let a = 3; let b = 4; a * a + b * b;");
    }

    #[test]
    fn res329_direct_recursion_matches_match() {
        // Hot, instruction-dense kernel — fib is the canonical
        // micro-benchmark for dispatch-overhead changes.
        let src =
            "fn fib(int n) { if n <= 1 { return n; } return fib(n - 1) + fib(n - 2); } fib(15);";
        let (m, d) = run_both(src);
        assert_eq!(value_repr(&m.unwrap()), value_repr(&d.unwrap()));
    }

    #[test]
    fn res329_direct_loops_match() {
        assert_both_eq("let i = 0; let sum = 0; while i < 50 { sum = sum + i; i = i + 1; } sum;");
    }

    #[test]
    fn res329_direct_arrays_match() {
        assert_both_eq("let a = [10, 20, 30]; a[0] + a[1] + a[2];");
        assert_both_eq("let a = [1, 2, 3]; a[1] = 99; return a[1];");
    }

    #[test]
    fn res329_direct_tail_call_matches() {
        let src = r#"
            fn sum(int acc, int n) {
                if n == 0 { return acc; }
                return sum(acc + n, n - 1);
            }
            sum(0, 1000);
        "#;
        let (m, d) = run_both(src);
        assert_eq!(value_repr(&m.unwrap()), value_repr(&d.unwrap()));
    }

    #[test]
    fn res329_direct_divide_by_zero_matches_match() {
        // Both paths must surface the SAME error kind with the SAME
        // line attribution (the AtLine wrapper is shared via run_with).
        let src = "let x = 10 / 0;";
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        let m = run_with(&prog, OverflowMode::Wrap, Dispatch::Match).unwrap_err();
        let d = run_with(&prog, OverflowMode::Wrap, Dispatch::Direct).unwrap_err();
        assert_eq!(m.kind(), d.kind());
        assert_eq!(m.kind(), &VmError::DivideByZero);
        // Both paths wrap with line info via the shared run_with outer.
        assert!(m.to_string().contains("line "));
        assert!(d.to_string().contains("line "));
    }

    #[test]
    fn res329_direct_struct_ops_match() {
        let src = "struct Point { int x, int y, } \
                   fn main() -> int { let p = new Point { x: 1, y: 42 }; return p.y; } \
                   main();";
        let (m, d) = run_both(src);
        assert_eq!(value_repr(&m.unwrap()), value_repr(&d.unwrap()));
    }

    #[test]
    fn res329_direct_overflow_traps_consistently() {
        // Trap mode must trip in both dispatch paths.
        let prog = const_program(
            &[Value::Int(i64::MAX), Value::Int(1)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        let m = run_with(&prog, OverflowMode::Trap, Dispatch::Match).unwrap_err();
        let d = run_with(&prog, OverflowMode::Trap, Dispatch::Direct).unwrap_err();
        assert_eq!(m.kind(), d.kind());
        assert_eq!(m.kind(), &VmError::IntegerOverflow("Add"));
    }

    #[test]
    fn res329_direct_call_stack_overflow_matches() {
        // Hand-rolled runaway recursion — the call-stack-overflow
        // diagnostic must surface from the direct-threaded handler too.
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
        let p = Program {
            main,
            functions: vec![runaway],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let err = run_with(&p, OverflowMode::Wrap, Dispatch::Direct).unwrap_err();
        assert_eq!(err.kind(), &VmError::CallStackOverflow);
    }

    #[test]
    fn res329_direct_unsupported_closures_surface_cleanly() {
        let p = const_program(
            &[],
            &[Op::MakeClosure {
                fn_idx: 0,
                upvalue_count: 0,
            }],
        );
        let err = run_with(&p, OverflowMode::Wrap, Dispatch::Direct).unwrap_err();
        match err.kind() {
            VmError::Unsupported(what) => assert_eq!(*what, "MakeClosure"),
            other => panic!("expected Unsupported(MakeClosure), got {:?}", other),
        }
    }

    #[test]
    fn res329_op_to_index_covers_every_variant() {
        // Smoke: each variant we feed into op_to_index must land on a
        // populated handler slot. Iterating a representative sample
        // (one of each variant shape) is enough — any mismatch lands
        // in `h_unreachable`.
        let samples: &[Op] = &[
            Op::Const(0),
            Op::Add,
            Op::Sub,
            Op::Mul,
            Op::Div,
            Op::Mod,
            Op::Neg,
            Op::LoadLocal(0),
            Op::StoreLocal(0),
            Op::Call(0),
            Op::ReturnFromCall,
            Op::Jump(0),
            Op::JumpIfFalse(0),
            Op::JumpIfTrue(0),
            Op::IncLocal(0),
            Op::Eq,
            Op::Neq,
            Op::Lt,
            Op::Le,
            Op::Gt,
            Op::Ge,
            Op::Not,
            Op::Return,
            Op::MakeClosure {
                fn_idx: 0,
                upvalue_count: 0,
            },
            Op::LoadUpvalue(0),
            Op::TailCall(0),
            Op::MakeArray { len: 0 },
            Op::LoadIndex,
            Op::StoreIndex,
            Op::CallForeign(0),
            Op::CallBuiltin {
                name_const: 0,
                arity: 0,
            },
            Op::StructLiteral {
                name_const: 0,
                field_count: 0,
            },
            Op::GetField { name_const: 0 },
            Op::SetField { name_const: 0 },
        ];
        for op in samples {
            let idx = op_to_index(*op);
            assert!(
                idx < HANDLER_TABLE_LEN,
                "op_to_index({:?}) = {} >= {}",
                op,
                idx,
                HANDLER_TABLE_LEN
            );
            // The handler slot is populated (not the unreachable sentinel)
            // — comparing fn pointers via address.
            let h = HANDLERS[idx];
            assert!(
                h as usize != h_unreachable as usize,
                "op {:?} mapped to unreachable slot {}",
                op,
                idx
            );
        }
    }
}
