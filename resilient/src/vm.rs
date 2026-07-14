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
    /// Shift amount is outside the valid range 0..63.
    /// Matches the tree-walker interpreter's `"shift amount out of range: N"` error.
    ShiftOutOfRange(i64),
    /// RES-break-continue: `assert cond[, msg];` fired at runtime
    /// (the condition was false). Carries the user-supplied or
    /// auto-generated failure message.
    AssertionFailed(String),
    /// RES-3996: `assume(cond[, msg]);` fired at runtime (the condition
    /// was false). Carries the user-supplied or auto-generated message.
    /// Kept distinct from `AssertionFailed` so the diagnostic reads
    /// "ASSUME VIOLATED" — matching the tree-walker's `eval_assume`.
    AssumeViolated(String),
    /// RES-169c: `LoadUpvalue(idx)` with `idx` outside the current
    /// frame's upvalue slab.
    UpvalueOutOfBounds(u16),
    /// RES-2544: a function with `fails` was called inside a try block.
    CheckedFailure(String),
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
            VmError::ShiftOutOfRange(n) => {
                write!(f, "shift amount out of range: {}", n)
            }
            VmError::AssertionFailed(msg) => {
                write!(f, "ASSERTION ERROR: {}", msg)
            }
            VmError::AssumeViolated(msg) => {
                write!(f, "ASSUME VIOLATED: {}", msg)
            }
            VmError::UpvalueOutOfBounds(idx) => {
                write!(f, "vm: upvalue index {} out of bounds", idx)
            }
            VmError::CheckedFailure(variant) => {
                write!(f, "vm: checked failure: {}", variant)
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

    fn div(self, a: i64, b: i64) -> Result<i64, VmError> {
        if b == 0 {
            return Err(VmError::DivideByZero);
        }
        if a == i64::MIN && b == -1 {
            return match self {
                OverflowMode::Wrap => Ok(a.wrapping_div(b)),
                OverflowMode::Saturate => Ok(i64::MAX),
                OverflowMode::Trap => Err(VmError::IntegerOverflow("Div")),
            };
        }
        Ok(a / b)
    }

    fn rem(self, a: i64, b: i64) -> Result<i64, VmError> {
        if b == 0 {
            return Err(VmError::DivideByZero);
        }
        if a == i64::MIN && b == -1 {
            return match self {
                OverflowMode::Wrap => Ok(0),
                OverflowMode::Saturate => Ok(0),
                OverflowMode::Trap => Err(VmError::IntegerOverflow("Mod")),
            };
        }
        Ok(a % b)
    }

    /// RES-349: tree-walker variant. The tree-walker interpreter in `lib.rs`
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

    pub fn div_for_eval(self, a: i64, b: i64) -> Result<i64, String> {
        if b == 0 {
            return Err("Division by zero".to_string());
        }
        if a == i64::MIN && b == -1 {
            return match self {
                OverflowMode::Wrap => Ok(a.wrapping_div(b)),
                OverflowMode::Saturate => Ok(i64::MAX),
                OverflowMode::Trap => Err(format!("integer overflow in / ({} / {})", a, b)),
            };
        }
        Ok(a / b)
    }

    pub fn rem_for_eval(self, a: i64, b: i64) -> Result<i64, String> {
        if b == 0 {
            return Err("Modulo by zero".to_string());
        }
        if a == i64::MIN && b == -1 {
            return match self {
                OverflowMode::Wrap | OverflowMode::Saturate => Ok(0),
                OverflowMode::Trap => Err(format!("integer overflow in % ({} % {})", a, b)),
            };
        }
        Ok(a % b)
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
const MAX_STRING_REPEAT: usize = 10_000_000;

/// RES-141 / RES-3995: process-wide live-block telemetry counters for
/// the VM backend, mirroring the tree-walker's `LIVE_TOTAL_RETRIES` /
/// `LIVE_TOTAL_EXHAUSTIONS` (lib.rs). Kept as separate VM-local statics
/// rather than sharing the interpreter's — each `rz` invocation only
/// ever runs one backend, so a fresh process gives each its own
/// zeroed counters either way, and this avoids reaching into the
/// interpreter's private thread-locals from a different module.
static VM_LIVE_TOTAL_RETRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static VM_LIVE_TOTAL_EXHAUSTIONS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// RES-359: per-retry sleep for a given backoff config and schedule
/// kind. Duplicates the tree-walker's private `live_backoff_delay_ms`
/// (lib.rs) — the math is a few lines of pure arithmetic over the
/// already-`pub` `BackoffConfig`/`BackoffKind` types, so re-deriving it
/// here is cheaper and lower-risk than reaching into a private
/// interpreter helper from a different module. Keep in sync with
/// lib.rs if the schedule shapes ever change.
fn vm_live_backoff_delay_ms(
    cfg: &crate::BackoffConfig,
    kind: crate::BackoffKind,
    retries: u32,
) -> u64 {
    match kind {
        crate::BackoffKind::Exponential => cfg.delay_ms(retries),
        crate::BackoffKind::Linear => {
            let steps = (retries as u64).saturating_add(1);
            let want = cfg.base_ms.saturating_mul(steps);
            want.min(cfg.max_ms)
        }
    }
}

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
    /// RES-169c: captured values for this closure frame. Empty for
    /// regular (non-closure) calls; `LoadUpvalue(i)` indexes here.
    upvalues: Box<[Value]>,
    /// RES-2536: absolute index into the `locals` slab where the
    /// closure value lives in the caller's frame. `None` for non-closure
    /// calls or temporary closures. Used by `ReturnFromCall` to write
    /// mutated upvalues back to the `Value::Closure`.
    closure_home: Option<usize>,
    /// RES-2536: for each upvalue, the caller-frame local slot it was
    /// captured from. On return, mutated upvalues are written back to
    /// both the `Value::Closure` and the caller's local slots.
    source_slots: Box<[u16]>,
}

/// RES-3914: write mutated upvalues back to the caller's locals and
/// the closure's own `Value::Closure` on return.
///
/// This is the pre-RES-3914 write-back mechanism (RES-2536): each
/// `StoreUpvalue` inside the callee updated `frame.upvalues` in place,
/// and on return that snapshot was copied into `caller_base + src` (the
/// *caller's* local slot the value was originally captured from) plus
/// the closure value's own upvalue slab. It assumed the closure never
/// outlives the frame it was captured in — true only for the
/// non-escaping, called-immediately case.
///
/// RES-3914 boxes every non-global captured variable into a shared
/// `Value::Cell` at capture time (see `compiler.rs`'s `BOXED_FLAG`),
/// so mutations go straight through `Cell.get()`/`Cell.set()` to the
/// thread-local cell store — visible to every closure and scope
/// sharing the capture without any write-back at all. `StoreUpvalue`
/// is consequently never emitted for a boxed capture, so
/// `popped.upvalues[i]` for those is an unchanging `Value::Cell`
/// handle. Skip write-back for `Value::Cell` entries: if the closure
/// has escaped its creating frame (returned out and called later),
/// `caller_base + src` refers to an unrelated slot in whatever frame
/// is now calling it, and writing the stale Cell handle there
/// corrupts that frame's own local (RES-3914's crash-on-returned-
/// counter, where it clobbered the very slot holding the closure
/// itself). Only captured *globals* — which are deliberately not
/// boxed, since `LoadGlobal`/`StoreGlobal` already address one shared
/// slot directly — can still reach this path with a non-`Cell` value.
fn write_back_upvalues(popped: &CallFrame, caller_base: usize, locals: &mut [Value]) {
    if popped.upvalues.is_empty() {
        return;
    }
    for (i, val) in popped.upvalues.iter().enumerate() {
        if matches!(val, Value::Cell(_)) {
            continue;
        }
        if let Some(&src) = popped.source_slots.get(i) {
            let abs = caller_base + src as usize;
            if abs < locals.len() {
                locals[abs] = val.clone();
            }
        }
    }
    if let Some(home) = popped.closure_home
        && let Some(Value::Closure { upvalues, .. }) = locals.get_mut(home)
    {
        *upvalues = popped.upvalues.clone();
    }
}

/// RES-2544: active try-catch handler frame. Pushed by `EnterTry`,
/// popped by `ExitTry`. When a `CheckedFailure` error fires during
/// a function call, the VM searches this stack for a matching handler.
#[derive(Debug, Clone)]
struct TryFrame {
    handler_table_idx: u16,
    chunk_idx: usize,
    call_depth: usize,
    stack_depth: usize,
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

/// RES-3995: which frame a live-block retry loop is tracking inside a
/// recursive `run_dispatch_loop` call, plus the attempt number the
/// *next* invocation should expose via `live_retries()`.
struct LiveTrack {
    /// Index into `frames` of the call that lexically contains the
    /// live block. Frames deeper than this belong to calls the body
    /// makes; once `frames.len()` drops to or below this value, the
    /// live block's own frame is gone.
    frame_idx: usize,
    /// Current attempt number (0 on the first attempt). Stays valid
    /// for the whole attempt, including any ordinary (non-live) calls
    /// the body makes — those don't recurse into a new
    /// `run_dispatch_loop` invocation, so `live_retries()` still
    /// resolves through this same `LiveTrack`.
    retry_count: usize,
}

/// RES-3995: how a (possibly nested) `run_dispatch_loop` invocation
/// ended.
enum LoopOutcome {
    /// The whole program halted (top-level `Op::Return`, or the
    /// implicit end-of-main fallthrough). Propagates through every
    /// active live-block retry loop unchanged — the program is
    /// ending, not just the innermost block.
    Halted(Value),
    /// Reached the paired `Op::ExitLive` for the tracked live block:
    /// this attempt's body ran to completion and any invariants held.
    ExitedNormally,
    /// The tracked live block's own enclosing frame returned from
    /// *inside* the body (an early `return`, e.g.
    /// `thermal_safety_cutoff.rz`'s `safe_read`). Also a success —
    /// there's no more of the block left to retry.
    ReturnedFromFrame(Value),
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
    // RES-1814: pre-size to 32 — typical VM runs have 5-30 locals
    // across main + frames. Same fixed-capacity shape as `stack` and
    // `frames`. The locals slot is grown via Op::AllocLocal as
    // frames push, so this is a starting capacity that absorbs the
    // common case without rehash churn.
    let mut locals: Vec<Value> = Vec::with_capacity(32);
    // RES-4046: per-function persistent storage for `static let`
    // bindings declared inside a `fn` body, indexed `[chunk_idx][slot]`
    // — unlike `locals`, this lives for the whole program run rather
    // than being reset every call, so a static's value survives across
    // separate invocations of the same function. `statics_init` tracks
    // which slots have already run their one-time initializer.
    let mut statics: Vec<Vec<Value>> = Vec::new();
    let mut statics_init: Vec<Vec<bool>> = Vec::new();
    let mut frames: Vec<CallFrame> = Vec::with_capacity(16);
    frames.push(CallFrame {
        chunk_idx: usize::MAX, // main
        pc: 0,
        locals_base: 0,
        upvalues: Box::default(),
        closure_home: None,
        source_slots: Box::default(),
    });
    let mut try_stack: Vec<TryFrame> = Vec::new();
    match run_dispatch_loop(
        program,
        &mut stack,
        &mut locals,
        &mut frames,
        &mut try_stack,
        &mut statics,
        &mut statics_init,
        overflow_mode,
        last_pc,
        None,
    )? {
        LoopOutcome::Halted(v) => Ok(v),
        // The top-level call always passes `tracked: None`, so the
        // ExitLive/ReturnedFromFrame stop conditions — which only
        // ever fire when `tracked.is_some()` — can't produce these.
        LoopOutcome::ExitedNormally | LoopOutcome::ReturnedFromFrame(_) => {
            unreachable!("top-level run_dispatch_loop only ever produces LoopOutcome::Halted")
        }
    }
}

/// RES-3995: the actual per-op dispatch loop, extracted from the
/// original `run_inner` so `Op::EnterLive` can recurse into it to run
/// a live block's body as an independently retryable unit — exactly
/// like the tree-walker's `eval_live_block` wraps `self.eval(body)` in
/// a Rust-level retry loop and catches `Err` normally (including
/// assertion failures deep inside a nested call, not just structured
/// `fails` errors). `tracked` is `Some` only for a recursive call
/// running a live block's body; `stack`/`locals`/`frames`/`try_stack`
/// are shared by `&mut` reference with the caller so a retry can see
/// (and roll back) whatever the failed attempt mutated.
#[allow(clippy::too_many_arguments)]
fn run_dispatch_loop(
    program: &Program,
    stack: &mut Vec<Value>,
    locals: &mut Vec<Value>,
    frames: &mut Vec<CallFrame>,
    try_stack: &mut Vec<TryFrame>,
    statics: &mut Vec<Vec<Value>>,
    statics_init: &mut Vec<Vec<bool>>,
    overflow_mode: OverflowMode,
    last_pc: &mut (usize, usize),
    tracked: Option<LiveTrack>,
) -> Result<LoopOutcome, VmError> {
    // RES-3995: once a frame pop drops `frames.len()` below this, the
    // tracked live block's own frame is gone — further ops belong to
    // whoever called it, and this invocation must stop.
    let target_frames_len = tracked.as_ref().map(|t| t.frame_idx + 1);
    let live_retry_count = tracked.as_ref().map(|t| t.retry_count);

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
                return Ok(LoopOutcome::Halted(stack.pop().unwrap_or(Value::Void)));
            }
            // In a fn body: implicit ReturnFromCall with Void.
            let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
            locals.truncate(popped.locals_base);
            stack.push(Value::Void);
            if let Some(target_len) = target_frames_len
                && frames.len() < target_len
            {
                return Ok(LoopOutcome::ReturnedFromFrame(Value::Void));
            }
            continue;
        }
        let op = chunk.code[pc];
        // RES-3995: the tracked live block's own paired `ExitLive` —
        // only at the exact frame depth the block was entered at
        // (nested live blocks in the same frame each get their own
        // recursive `run_dispatch_loop` call, so this always matches
        // the innermost still-open block first).
        if let Some(target_len) = target_frames_len
            && frames.len() == target_len
            && matches!(op, Op::ExitLive)
        {
            frames[frame_idx].pc += 1;
            return Ok(LoopOutcome::ExitedNormally);
        }
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
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => {
                        stack.push(Value::Int(overflow_mode.add(x, y, "Add")?));
                    }
                    (Value::Float(x), Value::Float(y)) => {
                        stack.push(Value::Float(x + y));
                    }
                    (Value::String(s1), Value::String(s2)) => {
                        stack.push(Value::String(s1 + &s2));
                    }
                    (Value::Array(mut l), Value::Array(r)) => {
                        l.extend(r);
                        stack.push(Value::Array(l));
                    }
                    (Value::String(mut s), ref rhs) if vm_can_stringify(rhs) => {
                        vm_push_stringified(&mut s, rhs);
                        stack.push(Value::String(s));
                    }
                    (ref lhs, Value::String(ref s)) if vm_can_stringify(lhs) => {
                        let mut buf = vm_stringify(lhs);
                        buf.push_str(s);
                        stack.push(Value::String(buf));
                    }
                    // RES-3994: `impl Add for T` operator-overload
                    // dispatch — mirrors `operator_overload::try_dispatch`
                    // (lib.rs). Pushes a `<StructName>$add` call frame
                    // instead of computing in place; its `Return`
                    // naturally lands the result back on the stack.
                    (a, b) => {
                        if let Some(fn_idx) = vm_operator_overload_fn_idx(program, "add", &a, &b) {
                            if frames.len() >= MAX_CALL_DEPTH {
                                return Err(VmError::CallStackOverflow);
                            }
                            let func = &program.functions[fn_idx];
                            let base = locals.len();
                            locals.resize(base + func.local_count as usize, Value::Void);
                            locals[base] = a;
                            locals[base + 1] = b;
                            frames.push(CallFrame {
                                chunk_idx: fn_idx,
                                pc: 0,
                                locals_base: base,
                                upvalues: Box::default(),
                                closure_home: None,
                                source_slots: Box::default(),
                            });
                            continue;
                        }
                        return Err(VmError::TypeMismatch("Add"));
                    }
                }
            }
            Op::Sub => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => {
                        stack.push(Value::Int(overflow_mode.sub(x, y, "Sub")?));
                    }
                    (Value::Float(x), Value::Float(y)) => {
                        stack.push(Value::Float(x - y));
                    }
                    // RES-3994: `impl Sub for T` operator overload (see Add above).
                    (a, b) => {
                        if let Some(fn_idx) = vm_operator_overload_fn_idx(program, "sub", &a, &b) {
                            if frames.len() >= MAX_CALL_DEPTH {
                                return Err(VmError::CallStackOverflow);
                            }
                            let func = &program.functions[fn_idx];
                            let base = locals.len();
                            locals.resize(base + func.local_count as usize, Value::Void);
                            locals[base] = a;
                            locals[base + 1] = b;
                            frames.push(CallFrame {
                                chunk_idx: fn_idx,
                                pc: 0,
                                locals_base: base,
                                upvalues: Box::default(),
                                closure_home: None,
                                source_slots: Box::default(),
                            });
                            continue;
                        }
                        return Err(VmError::TypeMismatch("Sub"));
                    }
                }
            }
            Op::Mul => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => {
                        stack.push(Value::Int(overflow_mode.mul(x, y, "Mul")?));
                    }
                    (Value::Float(x), Value::Float(y)) => {
                        stack.push(Value::Float(x * y));
                    }
                    (Value::String(ref s), Value::Int(n))
                    | (Value::Int(n), Value::String(ref s)) => {
                        if n < 0 {
                            return Err(VmError::BuiltinCallFailed(format!(
                                "string repetition count must be >= 0, got {}",
                                n
                            )));
                        }
                        let total = s.len().saturating_mul(n as usize);
                        if total > MAX_STRING_REPEAT {
                            return Err(VmError::BuiltinCallFailed(format!(
                                "string repeat: result length {} exceeds limit {}",
                                total, MAX_STRING_REPEAT
                            )));
                        }
                        stack.push(Value::String(s.repeat(n as usize)));
                    }
                    // RES-3994: `impl Mul for T` operator overload (see Add above).
                    (a, b) => {
                        if let Some(fn_idx) = vm_operator_overload_fn_idx(program, "mul", &a, &b) {
                            if frames.len() >= MAX_CALL_DEPTH {
                                return Err(VmError::CallStackOverflow);
                            }
                            let func = &program.functions[fn_idx];
                            let base = locals.len();
                            locals.resize(base + func.local_count as usize, Value::Void);
                            locals[base] = a;
                            locals[base + 1] = b;
                            frames.push(CallFrame {
                                chunk_idx: fn_idx,
                                pc: 0,
                                locals_base: base,
                                upvalues: Box::default(),
                                closure_home: None,
                                source_slots: Box::default(),
                            });
                            continue;
                        }
                        return Err(VmError::TypeMismatch("Mul"));
                    }
                }
            }
            Op::Div => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => {
                        stack.push(Value::Int(overflow_mode.div(x, y)?));
                    }
                    (Value::Float(x), Value::Float(y)) => {
                        stack.push(Value::Float(x / y));
                    }
                    _ => return Err(VmError::TypeMismatch("Div")),
                }
            }
            Op::Mod => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => {
                        stack.push(Value::Int(overflow_mode.rem(x, y)?));
                    }
                    (Value::Float(x), Value::Float(y)) => {
                        stack.push(Value::Float(x % y));
                    }
                    _ => return Err(VmError::TypeMismatch("Mod")),
                }
            }
            Op::Neg => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                match v {
                    Value::Int(i) => {
                        stack.push(Value::Int(overflow_mode.neg(i, "Neg")?));
                    }
                    Value::Float(f) => {
                        stack.push(Value::Float(-f));
                    }
                    _ => return Err(VmError::TypeMismatch("Neg")),
                }
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
                let func = program
                    .functions
                    .get(idx as usize)
                    .ok_or(VmError::FunctionOutOfBounds(idx))?;
                if !try_stack.is_empty() && !func.fails.is_empty() {
                    let variant = &func.fails[0];
                    let arity = func.arity as usize;
                    for _ in 0..arity {
                        stack.pop();
                    }
                    let mut dispatched = false;
                    while let Some(try_frame) = try_stack.pop() {
                        let handler_chunk = if try_frame.chunk_idx == usize::MAX {
                            &program.main
                        } else {
                            &program.functions[try_frame.chunk_idx].chunk
                        };
                        let entry =
                            &handler_chunk.try_handlers[try_frame.handler_table_idx as usize];
                        if let Some(arm) = entry.arms.iter().find(|a| a.variant == *variant) {
                            while frames.len() > try_frame.call_depth {
                                let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
                                locals.truncate(popped.locals_base);
                            }
                            stack.truncate(try_frame.stack_depth);
                            let fi = frames.len() - 1;
                            frames[fi].pc = arm.handler_pc;
                            dispatched = true;
                            break;
                        }
                    }
                    if dispatched {
                        // RES-3995: rare edge case — a `fails` catch
                        // dispatch unwound *past* a tracked live
                        // block's own frame (the handler lives in an
                        // ancestor of the live block, not inside it).
                        // The VM state already correctly resumes at
                        // the handler PC; this invocation just has
                        // nothing left to track, so stop immediately
                        // rather than let a stale `target_frames_len`
                        // misfire on later, unrelated ops.
                        if let Some(target_len) = target_frames_len
                            && frames.len() < target_len
                        {
                            return Ok(LoopOutcome::ReturnedFromFrame(Value::Void));
                        }
                        continue;
                    }
                    return Err(VmError::CheckedFailure(variant.clone()));
                }
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
                    upvalues: Box::default(),
                    closure_home: None,
                    source_slots: Box::default(),
                });
            }
            Op::ReturnFromCall => {
                // Pop the return value, unwind the frame, push it
                // onto the caller's stack. If the stack is empty the
                // function body ended without an expression — return
                // Void, matching the interpreter's implicit-return
                // semantics.
                let ret = stack.pop().unwrap_or(Value::Void);
                let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
                if frames.is_empty() {
                    return Ok(LoopOutcome::Halted(ret));
                }
                let caller_base = frames.last().map_or(0, |f| f.locals_base);
                write_back_upvalues(&popped, caller_base, locals);
                locals.truncate(popped.locals_base);
                stack.push(ret.clone());
                // RES-3995: an early `return` from inside a live
                // block's body pops the block's own enclosing frame
                // before ever reaching `ExitLive` — see
                // `thermal_safety_cutoff.rz`'s `safe_read`, whose
                // whole body is `live invariant true { ...; return
                // reading; }`. Treat it as the block succeeding.
                if let Some(target_len) = target_frames_len
                    && frames.len() < target_len
                {
                    return Ok(LoopOutcome::ReturnedFromFrame(ret));
                }
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
                    Value::Float(f) => f == 0.0,
                    Value::String(ref s) => s.is_empty(),
                    _ => false,
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
                    Value::Float(f) => f != 0.0,
                    Value::String(ref s) => !s.is_empty(),
                    _ => true,
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
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                stack.push(Value::Bool(vm_values_eq_checked(&a, &b)?));
            }
            Op::Neq => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                stack.push(Value::Bool(!vm_values_eq_checked(&a, &b)?));
            }
            Op::Lt => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => stack.push(Value::Bool(x < y)),
                    (Value::Float(x), Value::Float(y)) => stack.push(Value::Bool(x < y)),
                    (Value::String(ref x), Value::String(ref y)) => stack.push(Value::Bool(x < y)),
                    // RES-2683: char ordering.
                    (Value::Char(x), Value::Char(y)) => stack.push(Value::Bool(x < y)),
                    _ => return Err(VmError::TypeMismatch("Lt")),
                }
            }
            Op::Le => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => stack.push(Value::Bool(x <= y)),
                    (Value::Float(x), Value::Float(y)) => stack.push(Value::Bool(x <= y)),
                    (Value::String(ref x), Value::String(ref y)) => stack.push(Value::Bool(x <= y)),
                    // RES-2683: char ordering.
                    (Value::Char(x), Value::Char(y)) => stack.push(Value::Bool(x <= y)),
                    _ => return Err(VmError::TypeMismatch("Le")),
                }
            }
            Op::Gt => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => stack.push(Value::Bool(x > y)),
                    (Value::Float(x), Value::Float(y)) => stack.push(Value::Bool(x > y)),
                    (Value::String(ref x), Value::String(ref y)) => stack.push(Value::Bool(x > y)),
                    // RES-2683: char ordering.
                    (Value::Char(x), Value::Char(y)) => stack.push(Value::Bool(x > y)),
                    _ => return Err(VmError::TypeMismatch("Gt")),
                }
            }
            Op::Ge => {
                let b = stack.pop().ok_or(VmError::EmptyStack)?;
                let a = stack.pop().ok_or(VmError::EmptyStack)?;
                match (a, b) {
                    (Value::Int(x), Value::Int(y)) => stack.push(Value::Bool(x >= y)),
                    (Value::Float(x), Value::Float(y)) => stack.push(Value::Bool(x >= y)),
                    (Value::String(ref x), Value::String(ref y)) => stack.push(Value::Bool(x >= y)),
                    // RES-2683: char ordering.
                    (Value::Char(x), Value::Char(y)) => stack.push(Value::Bool(x >= y)),
                    _ => return Err(VmError::TypeMismatch("Ge")),
                }
            }
            Op::Not => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let negated = match v {
                    Value::Bool(b) => !b,
                    Value::Int(i) => i == 0,
                    Value::Float(f) => f == 0.0,
                    Value::String(ref s) => s.is_empty(),
                    _ => false,
                };
                stack.push(Value::Bool(negated));
            }
            Op::Return => {
                return Ok(LoopOutcome::Halted(stack.pop().unwrap_or(Value::Void)));
            }
            // RES-169a: skeleton dispatch arms. The compiler never
            // emits these until RES-169b lands the MakeClosure /
            // LoadUpvalue emission pass; if one shows up in a chunk
            // today it's a wiring bug, not user-facing. Return
            // Unsupported with a self-describing descriptor so the
            // at-line wrapper still works.
            Op::MakeClosure {
                fn_idx,
                upvalue_count,
            } => {
                if stack.len() < upvalue_count as usize {
                    return Err(VmError::EmptyStack);
                }
                let split = stack.len() - upvalue_count as usize;
                let captured: Box<[Value]> =
                    stack.drain(split..).collect::<Vec<_>>().into_boxed_slice();
                let src = program
                    .functions
                    .get(fn_idx as usize)
                    .map(|f| f.upvalue_source_slots.clone())
                    .unwrap_or_default();
                stack.push(Value::Closure {
                    fn_idx,
                    upvalues: captured,
                    source_slots: src,
                });
            }
            Op::LoadUpvalue(idx) => {
                let frame_idx = frames.len() - 1;
                let v = frames[frame_idx]
                    .upvalues
                    .get(idx as usize)
                    .ok_or(VmError::UpvalueOutOfBounds(idx))?
                    .clone();
                stack.push(v);
            }
            Op::StoreUpvalue {
                upvalue_idx,
                local_slot,
            } => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let frame_idx = frames.len() - 1;
                let frame = &mut frames[frame_idx];
                let abs = frame.locals_base + local_slot as usize;
                if locals.len() <= abs {
                    locals.resize(abs + 1, Value::Void);
                }
                locals[abs] = v.clone();
                if let Some(uv) = frame.upvalues.get_mut(upvalue_idx as usize) {
                    *uv = v;
                }
            }
            Op::CallClosure { arity, source_slot } => {
                if stack.len() < arity as usize + 1 {
                    return Err(VmError::EmptyStack);
                }
                if frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let caller_base = frames.last().map_or(0, |f| f.locals_base);
                let home = if source_slot != u16::MAX {
                    Some(caller_base + source_slot as usize)
                } else {
                    None
                };
                let split = stack.len() - arity as usize;
                let args: Vec<Value> = stack.drain(split..).collect();
                let closure = stack.pop().ok_or(VmError::EmptyStack)?;
                let (fn_idx, captured, src_slots) = match closure {
                    Value::Closure {
                        fn_idx,
                        upvalues,
                        source_slots,
                    } => (fn_idx, upvalues, source_slots),
                    // RES-3915: calling a first-class enum constructor value
                    // (`Color::Rgb` passed to a HOF, then invoked as `f(x)`)
                    // builds the corresponding EnumVariant, mirroring the
                    // interpreter's `apply_function` → `apply_constructor` path.
                    Value::EnumConstructor {
                        type_name,
                        variant,
                        arity: ctor_arity,
                    } => {
                        let result = crate::enum_ctors::apply_constructor(
                            &type_name, &variant, ctor_arity, args,
                        )
                        .map_err(|_| {
                            VmError::TypeMismatch("CallClosure: enum constructor arity mismatch")
                        })?;
                        stack.push(result);
                        continue;
                    }
                    _ => return Err(VmError::TypeMismatch("CallClosure: expected Closure")),
                };
                let func = program
                    .functions
                    .get(fn_idx as usize)
                    .ok_or(VmError::FunctionOutOfBounds(fn_idx))?;
                let base = locals.len();
                locals.resize(base + func.local_count as usize, Value::Void);
                for (i, v) in args.into_iter().enumerate() {
                    locals[base + i] = v;
                }
                frames.push(CallFrame {
                    chunk_idx: fn_idx as usize,
                    pc: 0,
                    locals_base: base,
                    upvalues: captured,
                    closure_home: home,
                    source_slots: src_slots,
                });
                continue;
            }
            Op::CallMethod {
                method_const,
                arity,
            } => {
                let method = match &chunk.constants[method_const as usize] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(VmError::TypeMismatch(
                            "CallMethod: bad method name constant",
                        ));
                    }
                };
                if stack.len() < arity as usize + 1 {
                    return Err(VmError::EmptyStack);
                }
                if frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let split = stack.len() - arity as usize;
                let args: Vec<Value> = stack.drain(split..).collect();
                let receiver = stack.pop().ok_or(VmError::EmptyStack)?;
                // RES-3994: primitive-`impl` receivers (`impl int { ... }`,
                // `impl float`, `impl string`, `impl bool` — RES-2553)
                // mangle the same way struct/enum methods do (`int$abs`).
                let mangled_prefix = match &receiver {
                    Value::Struct { name, .. } => Some(name.clone()),
                    Value::EnumVariant { type_name, .. } => Some(type_name.clone()),
                    other => vm_primitive_impl_type_name(other).map(str::to_string),
                };
                let Some(prefix) = mangled_prefix else {
                    // RES-3904: not a struct/enum/primitive-impl receiver
                    // — fall back to the built-in container method sugar
                    // (String/Array/Map/Set), which the compiler emits
                    // `CallMethod` for identically since it has no
                    // static type info.
                    let result = vm_call_builtin_method(receiver, &method, args)?;
                    stack.push(result);
                    continue;
                };
                let mangled = format!("{}${}", prefix, method);
                let Some(fn_idx) = program.functions.iter().position(|f| f.name == mangled) else {
                    // RES-3994: no matching `impl` method. Struct/enum
                    // receivers keep the existing hard error; primitive
                    // scalars fall back to the generic built-in method
                    // dispatch instead, mirroring the interpreter
                    // falling through past its primitive-impl check to
                    // the array-functional / generic builtin dispatch.
                    if matches!(&receiver, Value::Struct { .. } | Value::EnumVariant { .. }) {
                        return Err(VmError::TypeMismatch("CallMethod: method not found"));
                    }
                    let result = vm_call_builtin_method(receiver, &method, args)?;
                    stack.push(result);
                    continue;
                };
                let func = &program.functions[fn_idx];
                let base = locals.len();
                locals.resize(base + func.local_count as usize, Value::Void);
                locals[base] = receiver;
                for (i, v) in args.into_iter().enumerate() {
                    locals[base + 1 + i] = v;
                }
                frames.push(CallFrame {
                    chunk_idx: fn_idx,
                    pc: 0,
                    locals_base: base,
                    upvalues: Box::default(),
                    closure_home: None,
                    source_slots: Box::default(),
                });
                continue;
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
                // RES-1459: drain the contiguous top-`arity` span instead
                // of pop+reverse. Args were pushed left-to-right, so
                // `stack[len-arity..len]` is already in source order;
                // `drain` yields them in that order — no `reverse()`
                // needed. Same shape as the existing `MakeArray` arm
                // (line ~736).
                let split_at = stack.len() - arity;
                let args: Vec<crate::Value> = stack.drain(split_at..).collect();
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
                // RES-1421: hold the builtin name as `&str` borrowed
                // from the chunk's constant pool. The previous shape
                // cloned `s.clone()` into an owned `String` just to
                // pass `&name` into `lookup_builtin`, then cloned again
                // for the (rare) `UnknownBuiltin` error path. Builtin
                // dispatch fires on every `println` / `len` / etc.
                // call site — the per-call String alloc adds up.
                // After RES-1411 made `lookup_builtin` O(1) on `&str`,
                // dropping the alloc is the next obvious win. The
                // `chunk` borrow lives for the dispatch arm; the
                // `&str` is valid through `lookup_builtin` and the
                // `to_string()` on the `UnknownBuiltin` path.
                let name_val = chunk
                    .constants
                    .get(name_const as usize)
                    .ok_or(VmError::ConstantOutOfBounds(name_const))?;
                let name: &str = match name_val {
                    Value::String(s) => s.as_str(),
                    _ => return Err(VmError::TypeMismatch("CallBuiltin (non-string name)")),
                };
                let n = arity as usize;
                if stack.len() < n {
                    return Err(VmError::EmptyStack);
                }
                // RES-1459: drain instead of pop+reverse — see CallForeign
                // arm above for the same justification.
                let split_at = stack.len() - n;
                let args: Vec<Value> = stack.drain(split_at..).collect();
                // RES-3994: `to_string(struct_val)` dispatches to the
                // struct's `Display` impl (`<StructName>$fmt`) instead
                // of the generic scalar-only builtin — mirrors
                // `display_trait::try_display_fmt` (lib.rs). Push a
                // call frame instead of computing in place; the
                // frame's `Return` lands the formatted string back on
                // the stack.
                if let Some(fn_idx) = vm_display_fmt_fn_idx(program, name, &args) {
                    if frames.len() >= MAX_CALL_DEPTH {
                        return Err(VmError::CallStackOverflow);
                    }
                    let func = &program.functions[fn_idx];
                    let base = locals.len();
                    locals.resize(base + func.local_count as usize, Value::Void);
                    locals[base] = args.into_iter().next().expect("checked len == 1 above");
                    frames.push(CallFrame {
                        chunk_idx: fn_idx,
                        pc: 0,
                        locals_base: base,
                        upvalues: Box::default(),
                        closure_home: None,
                        source_slots: Box::default(),
                    });
                    continue;
                }
                // RES-3995: `live_retries()` / `live_total_retries()` /
                // `live_total_exhaustions()` read VM-local state
                // (which live block is active, and process-wide retry
                // counters) that the shared builtin table has no
                // access to — it only ever sees `&[Value]`. Intercept
                // these three names before the generic dispatch below;
                // everything else falls through unchanged.
                let result = if name == "live_retries" {
                    match live_retry_count {
                        Some(n) => Value::Int(n as i64),
                        None => {
                            return Err(VmError::BuiltinCallFailed(
                                "live_retries() called outside a live block".to_string(),
                            ));
                        }
                    }
                } else if name == "live_total_retries" {
                    Value::Int(
                        VM_LIVE_TOTAL_RETRIES.load(std::sync::atomic::Ordering::Relaxed) as i64,
                    )
                } else if name == "live_total_exhaustions" {
                    Value::Int(
                        VM_LIVE_TOTAL_EXHAUSTIONS.load(std::sync::atomic::Ordering::Relaxed) as i64,
                    )
                } else if let Some(func) = crate::lookup_builtin(name) {
                    func(&args).map_err(VmError::BuiltinCallFailed)?
                } else if let Some(stdlib_result) =
                    crate::stdlib::call_by_qualified_name(name, &args)
                {
                    stdlib_result.map_err(VmError::BuiltinCallFailed)?
                } else {
                    return Err(VmError::UnknownBuiltin(name.to_string()));
                };
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
            // RES-401: same convention as MakeArray — drain `len` values.
            Op::MakeTuple { len } => {
                let n = len as usize;
                if stack.len() < n {
                    return Err(VmError::EmptyStack);
                }
                let split_at = stack.len() - n;
                let items: Vec<Value> = stack.drain(split_at..).collect();
                stack.push(Value::Tuple(items));
            }
            // RES-375/RES-363: `expr?` — try-unwrap.
            Op::TryUnwrap => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                match v {
                    Value::Result { ok: true, payload } => {
                        stack.push(*payload);
                    }
                    Value::Option(Some(inner)) => {
                        stack.push(*inner);
                    }
                    Value::Result { ok: false, payload } => {
                        let ret = Value::Result { ok: false, payload };
                        let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
                        if frames.is_empty() {
                            return Ok(LoopOutcome::Halted(ret));
                        }
                        locals.truncate(popped.locals_base);
                        stack.push(ret.clone());
                        if let Some(target_len) = target_frames_len
                            && frames.len() < target_len
                        {
                            return Ok(LoopOutcome::ReturnedFromFrame(ret));
                        }
                    }
                    Value::Option(None) => {
                        let popped = frames.pop().ok_or(VmError::CallStackUnderflow)?;
                        if frames.is_empty() {
                            return Ok(LoopOutcome::Halted(Value::Option(None)));
                        }
                        locals.truncate(popped.locals_base);
                        stack.push(Value::Option(None));
                        if let Some(target_len) = target_frames_len
                            && frames.len() < target_len
                        {
                            return Ok(LoopOutcome::ReturnedFromFrame(Value::Option(None)));
                        }
                    }
                    _ => {
                        return Err(VmError::TypeMismatch(
                            "TryUnwrap: expected Result or Option",
                        ));
                    }
                }
            }
            Op::IterPrepare => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                stack.push(iter_prepare_value(v)?);
            }
            Op::LoadGlobal(idx) => {
                let abs = idx as usize;
                let v = locals
                    .get(abs)
                    .ok_or(VmError::LocalOutOfBounds(idx))?
                    .clone();
                stack.push(v);
            }
            Op::StoreGlobal(idx) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let abs = idx as usize;
                if locals.len() <= abs {
                    locals.resize(abs + 1, Value::Void);
                }
                locals[abs] = v;
            }
            // RES-4046: function-scoped `static let` — see the doc
            // comments on `Op::PushStaticInitialized` / `StoreStatic` /
            // `LoadStatic` in bytecode.rs. `statics`/`statics_init` are
            // declared once in `run_inner` and threaded through every
            // (possibly recursive, for `live` blocks) `run_dispatch_loop`
            // call, so they persist for the whole program run instead
            // of resetting per call like `locals` does.
            Op::PushStaticInitialized(idx) => {
                let chunk_idx = frames[frame_idx].chunk_idx;
                if chunk_idx == usize::MAX {
                    // The compiler never emits this at program scope —
                    // top-level `static let` compiles as a plain local
                    // (trivially "persistent": top-level code runs once).
                    return Err(VmError::Unsupported(
                        "static let initializer guard at program scope",
                    ));
                }
                if statics_init.len() <= chunk_idx {
                    statics_init.resize_with(chunk_idx + 1, Vec::new);
                }
                let flags = &mut statics_init[chunk_idx];
                let abs = idx as usize;
                if flags.len() <= abs {
                    flags.resize(abs + 1, false);
                }
                let already_initialized = flags[abs];
                flags[abs] = true;
                stack.push(Value::Bool(already_initialized));
            }
            Op::StoreStatic(idx) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let chunk_idx = frames[frame_idx].chunk_idx;
                if chunk_idx == usize::MAX {
                    return Err(VmError::Unsupported("static let store at program scope"));
                }
                if statics.len() <= chunk_idx {
                    statics.resize_with(chunk_idx + 1, Vec::new);
                }
                let slots = &mut statics[chunk_idx];
                let abs = idx as usize;
                if slots.len() <= abs {
                    slots.resize(abs + 1, Value::Void);
                }
                slots[abs] = v;
            }
            Op::LoadStatic(idx) => {
                let chunk_idx = frames[frame_idx].chunk_idx;
                if chunk_idx == usize::MAX {
                    return Err(VmError::Unsupported("static let load at program scope"));
                }
                let abs = idx as usize;
                let v = statics
                    .get(chunk_idx)
                    .and_then(|slots| slots.get(abs))
                    .cloned()
                    .unwrap_or(Value::Void);
                stack.push(v);
            }
            Op::EnterTry(handler_idx) => {
                try_stack.push(TryFrame {
                    handler_table_idx: handler_idx,
                    chunk_idx: frames[frames.len() - 1].chunk_idx,
                    call_depth: frames.len(),
                    stack_depth: stack.len(),
                });
            }
            Op::ExitTry => {
                try_stack.pop();
            }
            // RES-3995: `Op::ExitLive` is normally consumed by the
            // pre-check above (fired from the recursive call this
            // block's `EnterLive` made). Reaching this arm directly
            // would mean an `ExitLive` with no matching tracked
            // `EnterLive` at this frame depth — not reachable from any
            // program the compiler emits, but a no-op is safer than
            // an error if it ever is.
            Op::ExitLive => {}
            // RES-3995: `live { ... }` retry loop. The body was
            // compiled inline (same chunk, same locals slab) between
            // this op and its paired `ExitLive`; each attempt reruns
            // it via a recursive `run_dispatch_loop` call so any
            // error — including one raised deep inside a nested call —
            // is caught with plain Rust `Result` semantics, exactly
            // like the tree-walker's `eval_live_block`.
            Op::EnterLive(idx) => {
                let entry = chunk.live_handlers[idx as usize];
                let locals_base = frames[frame_idx].locals_base;
                let locals_snapshot: Vec<Value> = locals[locals_base..].to_vec();
                let stack_depth = stack.len();
                let max_retries = entry.max_retries as usize;
                let start_instant = entry.timeout_ns.map(|_| std::time::Instant::now());
                let mut retry_count: usize = 0;
                loop {
                    frames[frame_idx].pc = entry.body_start_pc;
                    let outcome = run_dispatch_loop(
                        program,
                        stack,
                        locals,
                        frames,
                        try_stack,
                        statics,
                        statics_init,
                        overflow_mode,
                        last_pc,
                        Some(LiveTrack {
                            frame_idx,
                            retry_count,
                        }),
                    );
                    match outcome {
                        Ok(LoopOutcome::ExitedNormally) => break,
                        Ok(LoopOutcome::ReturnedFromFrame(v)) => {
                            if let Some(target_len) = target_frames_len
                                && frames.len() < target_len
                            {
                                return Ok(LoopOutcome::ReturnedFromFrame(v));
                            }
                            break;
                        }
                        Ok(LoopOutcome::Halted(v)) => return Ok(LoopOutcome::Halted(v)),
                        Err(e) => {
                            // RES-359 + RES-141: same retry-count
                            // arithmetic as the tree-walker's
                            // `eval_live_block` — exhaustion fires
                            // once `retry_count` reaches
                            // `max_retries`, so `max_retries` is the
                            // total attempt count, not "initial +
                            // max_retries".
                            retry_count += 1;
                            if retry_count < max_retries {
                                VM_LIVE_TOTAL_RETRIES
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                            let timed_out = match (start_instant, entry.timeout_ns) {
                                (Some(t0), Some(budget)) => {
                                    t0.elapsed().as_nanos() >= u128::from(budget)
                                }
                                _ => false,
                            };
                            if retry_count >= max_retries || timed_out {
                                VM_LIVE_TOTAL_EXHAUSTIONS
                                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                return Err(e);
                            }
                            // RES-050: restore the pre-attempt
                            // locals/stack/frames snapshot — discards
                            // both the live block's own mutations and
                            // any leftover state from nested calls the
                            // failed attempt made, mirroring the
                            // tree-walker's env-snapshot restore.
                            frames.truncate(frame_idx + 1);
                            locals.truncate(locals_base);
                            locals.extend(locals_snapshot.iter().cloned());
                            stack.truncate(stack_depth);
                            if let Some(cfg) = &entry.backoff {
                                let ms = vm_live_backoff_delay_ms(
                                    cfg,
                                    entry.backoff_kind,
                                    (retry_count - 1) as u32,
                                );
                                if ms > 0 {
                                    crate::host_clock::sleep_ms(ms);
                                }
                            }
                        }
                    }
                }
            }
            Op::LoadIndex => {
                let idx_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let target = stack.pop().ok_or(VmError::EmptyStack)?;
                if let Value::Map(m) = target {
                    let mk =
                        crate::MapKey::from_value(&idx_val).map_err(VmError::BuiltinCallFailed)?;
                    let v = m.get(&mk).cloned().unwrap_or(Value::Void);
                    stack.push(v);
                } else {
                    let Value::Int(idx) = idx_val else {
                        return Err(VmError::TypeMismatch("LoadIndex (non-int index)"));
                    };
                    match target {
                        Value::Array(mut items) => {
                            let len = items.len() as i64;
                            let resolved = if idx < 0 { idx + len } else { idx };
                            if resolved < 0 || resolved >= len {
                                return Err(VmError::ArrayIndexOutOfBounds {
                                    index: idx,
                                    len: items.len(),
                                });
                            }
                            stack.push(items.swap_remove(resolved as usize));
                        }
                        Value::Tuple(mut items) => {
                            if idx < 0 || (idx as usize) >= items.len() {
                                return Err(VmError::ArrayIndexOutOfBounds {
                                    index: idx,
                                    len: items.len(),
                                });
                            }
                            stack.push(items.swap_remove(idx as usize));
                        }
                        Value::String(s) => {
                            let len = s.chars().count();
                            let len_i = len as i64;
                            let resolved = if idx < 0 { idx + len_i } else { idx };
                            if resolved < 0 || resolved >= len_i {
                                return Err(VmError::ArrayIndexOutOfBounds { index: idx, len });
                            }
                            let ch = s
                                .chars()
                                .nth(resolved as usize)
                                .expect("index already bounds-checked");
                            // RES-3889: yield `Value::Char` (not a single-char
                            // `Value::String`) to match the tree-walker
                            // interpreter (RES-2709). Without this, `s[i] == 'c'`
                            // and char-literal `match` arms silently diverge
                            // between the interpreter and the `--vm` backend.
                            stack.push(Value::Char(ch));
                        }
                        _ => return Err(VmError::TypeMismatch("LoadIndex (non-indexable target)")),
                    }
                }
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
                let Value::Array(mut items) = arr_val else {
                    return Err(VmError::TypeMismatch(
                        "LoadIndexUnchecked (non-array target)",
                    ));
                };
                let len = items.len() as i64;
                let resolved = if idx < 0 { idx + len } else { idx };
                if resolved < 0 || resolved >= len {
                    return Err(VmError::ArrayIndexOutOfBounds {
                        index: idx,
                        len: items.len(),
                    });
                }
                stack.push(items.swap_remove(resolved as usize));
            }
            Op::StoreIndex => {
                // Stack layout on entry (top → bottom):
                //   [v, idx, container, ...]
                // Pop in reverse-push order.
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let idx_val = stack.pop().ok_or(VmError::EmptyStack)?;
                let container = stack.pop().ok_or(VmError::EmptyStack)?;
                if let Value::Map(mut m) = container {
                    let mk =
                        crate::MapKey::from_value(&idx_val).map_err(VmError::BuiltinCallFailed)?;
                    m.insert(mk, v);
                    stack.push(Value::Map(m));
                } else {
                    let Value::Int(idx) = idx_val else {
                        return Err(VmError::TypeMismatch("StoreIndex (non-int index)"));
                    };
                    let Value::Array(mut items) = container else {
                        return Err(VmError::TypeMismatch("StoreIndex (non-array target)"));
                    };
                    let len = items.len() as i64;
                    let resolved = if idx < 0 { idx + len } else { idx };
                    if resolved < 0 || resolved >= len {
                        return Err(VmError::ArrayIndexOutOfBounds {
                            index: idx,
                            len: items.len(),
                        });
                    }
                    items[resolved as usize] = v;
                    stack.push(Value::Array(items));
                }
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
            Op::MakeEnumTuple {
                type_const,
                variant_const,
                arity,
            } => {
                let type_name = constant_as_string(chunk, type_const, "MakeEnumTuple (type)")?;
                let variant = constant_as_string(chunk, variant_const, "MakeEnumTuple (variant)")?;
                let n = arity as usize;
                if stack.len() < n {
                    return Err(VmError::EmptyStack);
                }
                let split_at = stack.len() - n;
                let vals: Vec<Value> = stack.drain(split_at..).collect();
                let payload = if vals.is_empty() {
                    crate::EnumValuePayload::None
                } else {
                    crate::EnumValuePayload::Tuple(vals)
                };
                stack.push(Value::EnumVariant {
                    type_name,
                    variant,
                    payload,
                });
            }
            Op::MakeEnumNamed {
                type_const,
                variant_const,
                field_count,
            } => {
                let type_name = constant_as_string(chunk, type_const, "MakeEnumNamed (type)")?;
                let variant = constant_as_string(chunk, variant_const, "MakeEnumNamed (variant)")?;
                let n = field_count as usize;
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
                        return Err(VmError::TypeMismatch("MakeEnumNamed (non-string key)"));
                    };
                    fields.push((field_name, v));
                }
                stack.push(Value::EnumVariant {
                    type_name,
                    variant,
                    payload: crate::EnumValuePayload::Named(fields),
                });
            }
            Op::GetField { name_const } => {
                // RES-1433: borrow field name from the constant pool
                // instead of cloning. The owned String was only ever
                // moved into the (rare) UnknownField error; the hot
                // success path compared `k == &field` and pushed the
                // found value without ever using ownership.
                let field = constant_as_str(chunk, name_const, "GetField (field name)")?;
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let val = vm_get_field_value(v, field)?;
                stack.push(val);
            }
            Op::SetField { name_const } => {
                // RES-1433: borrow field name from the constant pool —
                // same justification as the GetField arm above.
                let field = constant_as_str(chunk, name_const, "SetField (field name)")?;
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let tgt = stack.pop().ok_or(VmError::EmptyStack)?;
                let Value::Struct {
                    name: sname,
                    mut fields,
                } = tgt
                else {
                    return Err(VmError::TypeMismatch("SetField (non-struct target)"));
                };
                let slot = fields.iter_mut().find(|(k, _)| k.as_str() == field);
                match slot {
                    Some((_, existing)) => *existing = v,
                    None => {
                        return Err(VmError::UnknownField {
                            struct_name: sname,
                            field: field.to_string(),
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
            Op::Band => {
                let (a, b) = pop_two_ints(stack, "Band")?;
                stack.push(Value::Int(a & b));
            }
            Op::Bor => {
                let (a, b) = pop_two_ints(stack, "Bor")?;
                stack.push(Value::Int(a | b));
            }
            Op::Bxor => {
                let (a, b) = pop_two_ints(stack, "Bxor")?;
                stack.push(Value::Int(a ^ b));
            }
            Op::Shl => {
                let (a, b) = pop_two_ints(stack, "Shl")?;
                if !(0..64).contains(&b) {
                    return Err(VmError::ShiftOutOfRange(b));
                }
                stack.push(Value::Int(a << b));
            }
            Op::Shr => {
                let (a, b) = pop_two_ints(stack, "Shr")?;
                if !(0..64).contains(&b) {
                    return Err(VmError::ShiftOutOfRange(b));
                }
                stack.push(Value::Int(a >> b));
            }
            Op::AssertFail => {
                let msg = match stack.pop().ok_or(VmError::EmptyStack)? {
                    Value::String(s) => s,
                    other => format!("assertion failed: {}", other),
                };
                return Err(VmError::AssertionFailed(msg));
            }
            Op::AssumeFail => {
                let msg = match stack.pop().ok_or(VmError::EmptyStack)? {
                    Value::String(s) => s,
                    other => format!("assumption failed: {}", other),
                };
                return Err(VmError::AssumeViolated(msg));
            }
            Op::AssertBool => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                if !matches!(v, Value::Bool(_)) {
                    return Err(VmError::TypeMismatch("&& / || operand"));
                }
                stack.push(v);
            }
            // RES-3997: discard TOS — emitted after every expression-
            // statement whose value is unused so it doesn't leak into
            // whatever the next expression pops off the shared stack.
            Op::Pop => {
                stack.pop().ok_or(VmError::EmptyStack)?;
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

/// RES-1433: like [`constant_as_string`] but returns `&str` borrowed
/// directly from the constant pool. Use this when the caller only
/// needs to compare against the string or pass it on by reference;
/// avoids the per-dispatch `String::clone` that
/// `constant_as_string` pays even when the owned String is dropped
/// at the end of the arm.
///
/// The returned `&str` lives as long as the input `chunk` borrow,
/// which in the VM's dispatch loop is `&'p Chunk` borrowed from the
/// program — independent of the mutable `state.stack` borrow that
/// the surrounding handler does next.
fn constant_as_str<'a>(
    chunk: &'a Chunk,
    idx: u16,
    context: &'static str,
) -> Result<&'a str, VmError> {
    let v = chunk
        .constants
        .get(idx as usize)
        .ok_or(VmError::ConstantOutOfBounds(idx))?;
    match v {
        Value::String(s) => Ok(s.as_str()),
        _ => Err(VmError::TypeMismatch(context)),
    }
}

fn vm_can_stringify(v: &Value) -> bool {
    // RES-3889: `Value::Char` participates in `string + char` concatenation,
    // mirroring the interpreter's `can_stringify_for_concat`. String subscript
    // now yields a `Char`, so `"prefix" + s[i]` must stringify it.
    matches!(
        v,
        Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Char(_)
    )
}

fn vm_stringify(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
        // RES-3889: bare character, matching the interpreter's `Display`.
        Value::Char(c) => c.to_string(),
        _ => unreachable!(),
    }
}

fn vm_push_stringified(buf: &mut String, v: &Value) {
    match v {
        Value::Int(i) => buf.push_str(&i.to_string()),
        Value::Float(f) => buf.push_str(&f.to_string()),
        Value::Bool(b) => buf.push_str(&b.to_string()),
        // RES-3889: append the bare character, matching the interpreter.
        Value::Char(c) => buf.push(*c),
        _ => unreachable!(),
    }
}

/// RES-3994: mangled-method-name prefix for a `Value` under the
/// primitive-`impl` dispatch rules (`impl int { ... }`, `impl float
/// { ... }`, `impl string { ... }`, `impl bool { ... }` — RES-2553).
/// Mirrors the interpreter's `prim_type_name` (lib.rs, `CallExpression`
/// eval): the parser stores `impl int` with `struct_name == "int"`, so
/// methods register as `int$method_name` and dispatch the same way
/// struct/enum methods do. Returns `None` for receivers that aren't a
/// primitive-impl-eligible scalar (Array/Map/Set/etc. never have user
/// `impl` blocks — they only get the built-in container methods below).
fn vm_primitive_impl_type_name(v: &Value) -> Option<&'static str> {
    match v {
        Value::Int(_) => Some("int"),
        Value::Float(_) => Some("float"),
        Value::String(_) => Some("string"),
        Value::Bool(_) => Some("bool"),
        _ => None,
    }
}

/// RES-3994: operator-overload dispatch for `Op::Add` / `Op::Sub` /
/// `Op::Mul` — mirrors `operator_overload::try_dispatch` (lib.rs),
/// which the tree-walking interpreter consults before raising a type
/// mismatch. When either operand is a `Value::Struct` and the program
/// defines the conventional `<StructName>$<method>` function (`add`,
/// `sub`, `mul` — same mapping as `operator_overload::op_method_name`),
/// return its function index so the caller can push a call frame
/// instead of erroring. `None` means "no overload — raise the usual
/// `TypeMismatch`", matching the interpreter falling through to its
/// own type-mismatch error when no method is registered.
fn vm_operator_overload_fn_idx(
    program: &Program,
    method: &'static str,
    left: &Value,
    right: &Value,
) -> Option<usize> {
    let struct_name = match left {
        Value::Struct { name, .. } => name,
        _ => match right {
            Value::Struct { name, .. } => name,
            _ => return None,
        },
    };
    let mangled = format!("{}${}", struct_name, method);
    program.functions.iter().position(|f| f.name == mangled)
}

/// RES-3994: `to_string(x)` free-function dispatch to a struct's
/// `Display` impl — mirrors `display_trait::try_display_fmt` (lib.rs),
/// which the interpreter's `to_string` builtin call site consults
/// before falling back to `builtin_to_string`'s scalar-only handling.
/// Returns the `<StructName>$fmt` function index when `args` is
/// exactly one `Value::Struct` with a registered `Display` impl;
/// `None` otherwise (caller falls through to the generic builtin,
/// which raises "expected scalar value" for a struct with no impl —
/// same error the interpreter surfaces).
fn vm_display_fmt_fn_idx(program: &Program, name: &str, args: &[Value]) -> Option<usize> {
    if name != "to_string" {
        return None;
    }
    let [
        Value::Struct {
            name: struct_name, ..
        },
    ] = args
    else {
        return None;
    };
    let mangled = format!("{}$fmt", struct_name);
    program.functions.iter().position(|f| f.name == mangled)
}

/// RES-3904: `Op::CallMethod` fallback for built-in container
/// receivers (`String`/`Array`/`Map`/`Set`). The compiler emits
/// `CallMethod` for *every* `x.y(...)` dot-call uniformly (it has no
/// static type info at that point), so `s.to_upper()` reaches the same
/// opcode as a struct method call. Looks up the full builtin name via
/// the same `builtin_method_full_name` table the interpreter uses
/// (`lib.rs`), then dispatches through `lookup_builtin` — the exact
/// mechanism `Op::CallBuiltin` already uses. `Array::collect()` is an
/// identity, not a builtin call, and is handled before the shared
/// lookup, mirroring the interpreter.
///
/// RES-3994: also handles `Result` error-chaining methods (`.context()`,
/// `.root_cause()`, `.chain()`) by delegating to the same pure
/// `error_chaining::dispatch_result_method` the interpreter uses —
/// no interpreter state is needed, so the VM can call it directly.
fn vm_call_builtin_method(
    receiver: Value,
    method: &str,
    args: Vec<Value>,
) -> Result<Value, VmError> {
    if let Value::Array(_) = &receiver
        && method == "collect"
    {
        if !args.is_empty() {
            return Err(VmError::TypeMismatch("collect: expected 0 arguments"));
        }
        return Ok(receiver);
    }
    // RES-3914: `Value::Cell` upvalue-box method dispatch. The compiler
    // routes reads/writes of a captured-mutable variable through
    // `Cell.get()` / `Cell.set()` (see `compiler.rs`'s `BOXED_FLAG`) so
    // every closure sharing the capture — and the defining scope
    // itself — observes the same mutations instead of each closure
    // holding an independent snapshot. Mirrors the interpreter's
    // `Value::Cell` method dispatch in `lib.rs` (RES-328) by calling
    // the same private `cell_get`/`cell_set` helpers directly; `Cell`
    // isn't a struct/enum so it can't go through the
    // `builtin_method_full_name` table below.
    if let Value::Cell(id) = receiver {
        return match method {
            "get" => {
                if !args.is_empty() {
                    return Err(VmError::TypeMismatch("Cell.get: expected 0 arguments"));
                }
                crate::cell_get(id).map_err(VmError::BuiltinCallFailed)
            }
            "set" => match args.as_slice() {
                [v] => crate::cell_set(id, v.clone()).map_err(VmError::BuiltinCallFailed),
                _ => Err(VmError::TypeMismatch("Cell.set: expected 1 argument")),
            },
            _ => Err(VmError::TypeMismatch("Cell: unknown method")),
        };
    }
    // RES-3994: `Result` error-chaining methods — `.context(msg)`,
    // `.root_cause()`, `.chain()`. Pure function of (ok, payload,
    // method, args); no interpreter state needed, so the same
    // `error_chaining::dispatch_result_method` the tree-walker uses
    // can be called directly here.
    if let Value::Result { ok, ref payload } = receiver
        && let Some(result) =
            crate::error_chaining::dispatch_result_method(ok, payload, method, &args)
    {
        return result.map_err(VmError::BuiltinCallFailed);
    }
    let full_name = crate::builtin_method_full_name(&receiver, method).ok_or(
        VmError::TypeMismatch("CallMethod: receiver is not a struct or enum"),
    )?;
    let mut call_args = Vec::with_capacity(args.len() + 1);
    call_args.push(receiver);
    call_args.extend(args);
    let func = crate::lookup_builtin(full_name)
        .ok_or(VmError::TypeMismatch("CallMethod: builtin not registered"))?;
    func(&call_args).map_err(VmError::BuiltinCallFailed)
}

fn vm_values_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        // RES-2683: char equality.
        (Value::Char(x), Value::Char(y)) => x == y,
        (Value::Void, Value::Void) => true,
        (Value::Array(x), Value::Array(y)) | (Value::Tuple(x), Value::Tuple(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| vm_values_eq(a, b))
        }
        (
            Value::Struct {
                name: ln,
                fields: lf,
            },
            Value::Struct {
                name: rn,
                fields: rf,
            },
        ) => {
            ln == rn
                && lf.len() == rf.len()
                && lf.iter().all(|(name, lv)| {
                    rf.iter()
                        .find(|(rn, _)| rn == name)
                        .is_some_and(|(_, rv)| vm_values_eq(lv, rv))
                })
        }
        (Value::Map(lm), Value::Map(rm)) => {
            lm.len() == rm.len()
                && lm
                    .iter()
                    .all(|(k, lv)| rm.get(k).is_some_and(|rv| vm_values_eq(lv, rv)))
        }
        // RES-3998: Set equality — mirrors the interpreter's `values_strict_eq`
        // `Value::Set` arm. `MapKey` is `Eq`/`Hash`, so `HashSet::eq` (set
        // membership, order-independent) is exactly what the interpreter does.
        (Value::Set(ls), Value::Set(rs)) => ls == rs,
        // RES-3998: Option equality — mirrors the interpreter's recursive
        // `Value::Option` arm (RES-2723). Without this, `Some(5) == Some(5)`
        // and `None == None` fell through to `_ => false` under `--vm`.
        (Value::Option(l), Value::Option(r)) => match (l.as_deref(), r.as_deref()) {
            (None, None) => true,
            (Some(lv), Some(rv)) => vm_values_eq(lv, rv),
            _ => false,
        },
        // RES-3998: Result equality — mirrors the interpreter's recursive
        // `Value::Result` arm (RES-2726). `Ok(x) == Ok(y)` iff `x == y`;
        // an `ok` mismatch (Ok vs Err) is unequal.
        (
            Value::Result {
                ok: lok,
                payload: lp,
            },
            Value::Result {
                ok: rok,
                payload: rp,
            },
        ) => lok == rok && vm_values_eq(lp, rp),
        // RES-3916: enum-variant equality — same type, same variant, same
        // payload — mirroring the interpreter's `values_strict_eq`
        // EnumVariant arm. Without this, two equal variants (e.g. bare
        // `E::A == E::A`, now reachable on the VM) fell through to
        // `_ => false` and compared unequal.
        (
            Value::EnumVariant {
                type_name: ltn,
                variant: lv,
                payload: lp,
            },
            Value::EnumVariant {
                type_name: rtn,
                variant: rv,
                payload: rp,
            },
        ) => ltn == rtn && lv == rv && vm_enum_payload_eq(lp, rp),
        _ => false,
    }
}

/// RES-3916: structural equality for enum-variant payloads, mirroring the
/// interpreter's `enum_payload_strict_eq`. Nested values recurse through
/// `vm_values_eq` so the two backends agree on compound payloads.
fn vm_enum_payload_eq(l: &crate::EnumValuePayload, r: &crate::EnumValuePayload) -> bool {
    use crate::EnumValuePayload::{Named, None as PNone, Tuple};
    match (l, r) {
        (PNone, PNone) => true,
        (Tuple(lv), Tuple(rv)) => {
            lv.len() == rv.len() && lv.iter().zip(rv.iter()).all(|(a, b)| vm_values_eq(a, b))
        }
        (Named(lf), Named(rf)) => {
            lf.len() == rf.len()
                && lf.iter().all(|(name, lval)| {
                    rf.iter()
                        .find(|(rn, _)| rn == name)
                        .is_some_and(|(_, rval)| vm_values_eq(lval, rval))
                })
        }
        _ => false,
    }
}

/// RES-3891: top-level `==` / `!=` must reject operands of different kinds the
/// same way the tree-walking interpreter does — with a runtime type mismatch —
/// rather than silently reporting them unequal via `vm_values_eq`'s `_ => false`
/// catch-all. This mirrors the interpreter's split between `eval_infix` (which
/// errors at the outermost comparison) and `values_strict_eq` (which stays total
/// for elements *nested inside* compound values). Only the outermost comparison
/// is checked here; `vm_values_eq` is unchanged, so structural equality inside
/// arrays, structs, and maps remains total on both backends.
fn vm_values_eq_checked(a: &Value, b: &Value) -> Result<bool, VmError> {
    if std::mem::discriminant(a) == std::mem::discriminant(b) {
        Ok(vm_values_eq(a, b))
    } else {
        Err(VmError::TypeMismatch("=="))
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
    /// RES-2544: a checked failure was injected. The dispatch loop
    /// unwinds to the matching try-catch handler.
    CatchDispatch(String),
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
    try_stack: Vec<TryFrame>,
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
const OP_KIND_COUNT: usize = 39;

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
        Op::MakeEnumTuple { .. } => OP_KIND_STRUCT_LITERAL,
        Op::MakeEnumNamed { .. } => OP_KIND_STRUCT_LITERAL,
        Op::GetField { .. } => OP_KIND_GET_FIELD,
        Op::SetField { .. } => OP_KIND_SET_FIELD,
        Op::Band => OP_KIND_BAND,
        Op::Bor => OP_KIND_BOR,
        Op::Bxor => OP_KIND_BXOR,
        Op::Shl => OP_KIND_SHL,
        Op::Shr => OP_KIND_SHR,
        Op::AssertFail => OP_KIND_ASSERT_FAIL,
        Op::MakeTuple { .. } => OP_KIND_MAKE_TUPLE,
        Op::CallClosure { .. } => OP_KIND_CALL_CLOSURE,
        Op::TryUnwrap => OP_KIND_TRY_UNWRAP,
        Op::IterPrepare => OP_KIND_ITER_PREPARE,
        Op::LoadGlobal(_) => OP_KIND_LOAD_GLOBAL,
        Op::StoreGlobal(_) => OP_KIND_STORE_GLOBAL,
        Op::StoreUpvalue { .. } => OP_KIND_STORE_UPVALUE,
        Op::CallMethod { .. } => OP_KIND_CALL_METHOD,
        Op::EnterTry(_) => OP_KIND_ENTER_TRY,
        Op::ExitTry => OP_KIND_EXIT_TRY,
        Op::AssertBool => OP_KIND_ASSERT_BOOL,
        Op::Pop => OP_KIND_POP,
        Op::AssumeFail => OP_KIND_ASSUME_FAIL,
        Op::EnterLive(_) => OP_KIND_ENTER_LIVE,
        Op::ExitLive => OP_KIND_EXIT_LIVE,
        Op::PushStaticInitialized(_) => OP_KIND_PUSH_STATIC_INITIALIZED,
        Op::StoreStatic(_) => OP_KIND_STORE_STATIC,
        Op::LoadStatic(_) => OP_KIND_LOAD_STATIC,
    }
}

const OP_KIND_LOAD_INDEX_UNCHECKED: usize = 31;
const OP_KIND_STRUCT_LITERAL: usize = 32;
const OP_KIND_GET_FIELD: usize = 33;
const OP_KIND_SET_FIELD: usize = 34;
const OP_KIND_BAND: usize = 35;
const OP_KIND_BOR: usize = 36;
const OP_KIND_BXOR: usize = 37;
const OP_KIND_SHL: usize = 38;
const OP_KIND_SHR: usize = 39;
const OP_KIND_ASSERT_FAIL: usize = 40;
const OP_KIND_MAKE_TUPLE: usize = 41;
const OP_KIND_CALL_CLOSURE: usize = 42;
const OP_KIND_TRY_UNWRAP: usize = 43;
const OP_KIND_ITER_PREPARE: usize = 44;
const OP_KIND_LOAD_GLOBAL: usize = 45;
const OP_KIND_STORE_GLOBAL: usize = 46;
const OP_KIND_STORE_UPVALUE: usize = 47;
const OP_KIND_CALL_METHOD: usize = 48;
const OP_KIND_ENTER_TRY: usize = 49;
const OP_KIND_EXIT_TRY: usize = 50;
const OP_KIND_ASSERT_BOOL: usize = 51;
const OP_KIND_POP: usize = 52;
const OP_KIND_ASSUME_FAIL: usize = 53;
/// RES-3995: `EnterLive`/`ExitLive` are only implemented by the Match
/// dispatch engine (`run_inner`); the Direct (table-dispatch) engine
/// surfaces a clean `VmError::Unsupported` via `h_live_unsupported`
/// instead of silently mis-executing the block. Follow-up ticket:
/// port the retry-loop recursion to `run_direct`.
const OP_KIND_ENTER_LIVE: usize = 54;
const OP_KIND_EXIT_LIVE: usize = 55;
/// RES-4046: function-scoped `static let` persistence is only
/// implemented by the Match dispatch engine (`run_inner`) — it needs
/// the `statics`/`statics_init` tables threaded through
/// `run_dispatch_loop`, which `run_direct` doesn't share. The Direct
/// engine surfaces a clean `VmError::Unsupported` via
/// `h_static_unsupported` instead of silently resetting the static on
/// every call (this ticket's original bug). Follow-up: port the
/// statics tables to `VmState` so `run_direct` gets full parity.
const OP_KIND_PUSH_STATIC_INITIALIZED: usize = 56;
const OP_KIND_STORE_STATIC: usize = 57;
const OP_KIND_LOAD_STATIC: usize = 58;
const HANDLER_TABLE_LEN: usize = 59;

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
    table[OP_KIND_BAND] = h_band;
    table[OP_KIND_BOR] = h_bor;
    table[OP_KIND_BXOR] = h_bxor;
    table[OP_KIND_SHL] = h_shl;
    table[OP_KIND_SHR] = h_shr;
    table[OP_KIND_ASSERT_FAIL] = h_assert_fail;
    table[OP_KIND_MAKE_TUPLE] = h_make_tuple;
    table[OP_KIND_CALL_CLOSURE] = h_call_closure;
    table[OP_KIND_TRY_UNWRAP] = h_try_unwrap;
    table[OP_KIND_ITER_PREPARE] = h_iter_prepare;
    table[OP_KIND_LOAD_GLOBAL] = h_load_global;
    table[OP_KIND_STORE_GLOBAL] = h_store_global;
    table[OP_KIND_STORE_UPVALUE] = h_store_upvalue;
    table[OP_KIND_CALL_METHOD] = h_call_method;
    table[OP_KIND_ENTER_TRY] = h_enter_try;
    table[OP_KIND_EXIT_TRY] = h_exit_try;
    table[OP_KIND_ASSERT_BOOL] = h_assert_bool;
    table[OP_KIND_POP] = h_pop;
    table[OP_KIND_ASSUME_FAIL] = h_assume_fail;
    table[OP_KIND_ENTER_LIVE] = h_live_unsupported;
    table[OP_KIND_EXIT_LIVE] = h_live_unsupported;
    table[OP_KIND_PUSH_STATIC_INITIALIZED] = h_static_unsupported;
    table[OP_KIND_STORE_STATIC] = h_static_unsupported;
    table[OP_KIND_LOAD_STATIC] = h_static_unsupported;
    table
};

/// RES-3995: `run_direct` doesn't implement live-block retry semantics
/// yet — see the `OP_KIND_ENTER_LIVE` doc comment. Surface a clean
/// error instead of silently running the body exactly once (which
/// would print correct output on the happy path but corrupt retry
/// counters / skip the retry loop on the failure path).
#[inline(never)]
fn h_live_unsupported(_state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Err(VmError::Unsupported(
        "live block (RESILIENT_DISPATCH=direct doesn't implement RES-3995 retry semantics yet)",
    ))
}

/// RES-4046: `run_direct` doesn't implement function-scoped `static
/// let` persistence yet — see the `OP_KIND_PUSH_STATIC_INITIALIZED`
/// doc comment. Surface a clean error instead of silently resetting
/// the static's value on every call (the bug this ticket fixes).
#[inline(never)]
fn h_static_unsupported(_state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Err(VmError::Unsupported(
        "static let (RESILIENT_DISPATCH=direct doesn't implement RES-4046 persistence yet)",
    ))
}

/// Direct-threaded entry point. Mirrors `run_inner` byte-for-byte but
/// dispatches via the handler table. The outer error-wrapping logic
/// in `run_with` is shared, so `last_pc` updates are identical.
fn run_direct(
    program: &Program,
    last_pc: &mut (usize, usize),
    overflow_mode: OverflowMode,
) -> Result<Value, VmError> {
    // RES-1830: pre-size `locals` to 32 — mirrors RES-1814 for the
    // tree-walker `run_inner` path. `run_direct` is the second VM
    // entry point and pays the same 0→4→… doubling chain without
    // pre-sizing.
    let mut state = VmState {
        program,
        stack: Vec::with_capacity(64),
        locals: Vec::with_capacity(32),
        frames: Vec::with_capacity(16),
        overflow_mode,
        try_stack: Vec::new(),
    };
    state.frames.push(CallFrame {
        chunk_idx: usize::MAX,
        pc: 0,
        locals_base: 0,
        upvalues: Box::default(),
        closure_home: None,
        source_slots: Box::default(),
    });

    loop {
        let frame_idx = state.frame_idx();
        let chunk = state.current_chunk();
        let pc = state.frames[frame_idx].pc;
        *last_pc = (state.frames[frame_idx].chunk_idx, pc + 1);

        if pc >= chunk.code.len() {
            if state.frames.len() == 1 {
                return Ok(state.stack.pop().unwrap_or(Value::Void));
            }
            let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
            let caller_base = state.frames.last().map_or(0, |f| f.locals_base);
            write_back_upvalues(&popped, caller_base, &mut state.locals);
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
            Step::CatchDispatch(variant) => {
                let mut dispatched = false;
                while let Some(try_frame) = state.try_stack.pop() {
                    let handler_chunk = if try_frame.chunk_idx == usize::MAX {
                        &state.program.main
                    } else {
                        &state.program.functions[try_frame.chunk_idx].chunk
                    };
                    let entry = &handler_chunk.try_handlers[try_frame.handler_table_idx as usize];
                    if let Some(arm) = entry.arms.iter().find(|a| a.variant == variant) {
                        while state.frames.len() > try_frame.call_depth {
                            let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
                            state.locals.truncate(popped.locals_base);
                        }
                        state.stack.truncate(try_frame.stack_depth);
                        let fi = state.frame_idx();
                        state.frames[fi].pc = arm.handler_pc;
                        dispatched = true;
                        break;
                    }
                }
                if dispatched {
                    continue;
                }
                return Err(VmError::CheckedFailure(variant));
            }
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
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            state
                .stack
                .push(Value::Int(state.overflow_mode.add(x, y, "Add")?));
        }
        (Value::Float(x), Value::Float(y)) => {
            state.stack.push(Value::Float(x + y));
        }
        (Value::String(s1), Value::String(s2)) => {
            state.stack.push(Value::String(s1 + &s2));
        }
        (Value::Array(mut l), Value::Array(r)) => {
            l.extend(r);
            state.stack.push(Value::Array(l));
        }
        (Value::String(mut s), ref rhs) if vm_can_stringify(rhs) => {
            vm_push_stringified(&mut s, rhs);
            state.stack.push(Value::String(s));
        }
        (ref lhs, Value::String(ref s)) if vm_can_stringify(lhs) => {
            let mut buf = vm_stringify(lhs);
            buf.push_str(s);
            state.stack.push(Value::String(buf));
        }
        // RES-3994: `impl Add for T` operator-overload dispatch — see
        // the `run_inner` `Op::Add` arm for the full rationale; this
        // is the direct-threaded-dispatch twin, kept byte-identical.
        (a, b) => {
            if let Some(fn_idx) = vm_operator_overload_fn_idx(state.program, "add", &a, &b) {
                if state.frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let func = &state.program.functions[fn_idx];
                let base = state.locals.len();
                state
                    .locals
                    .resize(base + func.local_count as usize, Value::Void);
                state.locals[base] = a;
                state.locals[base + 1] = b;
                state.frames.push(CallFrame {
                    chunk_idx: fn_idx,
                    pc: 0,
                    locals_base: base,
                    upvalues: Box::default(),
                    closure_home: None,
                    source_slots: Box::default(),
                });
                return Ok(Step::Continue);
            }
            return Err(VmError::TypeMismatch("Add"));
        }
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_sub(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            state
                .stack
                .push(Value::Int(state.overflow_mode.sub(x, y, "Sub")?));
        }
        (Value::Float(x), Value::Float(y)) => {
            state.stack.push(Value::Float(x - y));
        }
        // RES-3994: `impl Sub for T` operator overload (see h_add above).
        (a, b) => {
            if let Some(fn_idx) = vm_operator_overload_fn_idx(state.program, "sub", &a, &b) {
                if state.frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let func = &state.program.functions[fn_idx];
                let base = state.locals.len();
                state
                    .locals
                    .resize(base + func.local_count as usize, Value::Void);
                state.locals[base] = a;
                state.locals[base + 1] = b;
                state.frames.push(CallFrame {
                    chunk_idx: fn_idx,
                    pc: 0,
                    locals_base: base,
                    upvalues: Box::default(),
                    closure_home: None,
                    source_slots: Box::default(),
                });
                return Ok(Step::Continue);
            }
            return Err(VmError::TypeMismatch("Sub"));
        }
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_mul(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            state
                .stack
                .push(Value::Int(state.overflow_mode.mul(x, y, "Mul")?));
        }
        (Value::Float(x), Value::Float(y)) => {
            state.stack.push(Value::Float(x * y));
        }
        (Value::String(ref s), Value::Int(n)) | (Value::Int(n), Value::String(ref s)) => {
            if n < 0 {
                return Err(VmError::BuiltinCallFailed(format!(
                    "string repetition count must be >= 0, got {}",
                    n
                )));
            }
            let total = s.len().saturating_mul(n as usize);
            if total > MAX_STRING_REPEAT {
                return Err(VmError::BuiltinCallFailed(format!(
                    "string repeat: result length {} exceeds limit {}",
                    total, MAX_STRING_REPEAT
                )));
            }
            state.stack.push(Value::String(s.repeat(n as usize)));
        }
        // RES-3994: `impl Mul for T` operator overload (see h_add above).
        (a, b) => {
            if let Some(fn_idx) = vm_operator_overload_fn_idx(state.program, "mul", &a, &b) {
                if state.frames.len() >= MAX_CALL_DEPTH {
                    return Err(VmError::CallStackOverflow);
                }
                let func = &state.program.functions[fn_idx];
                let base = state.locals.len();
                state
                    .locals
                    .resize(base + func.local_count as usize, Value::Void);
                state.locals[base] = a;
                state.locals[base + 1] = b;
                state.frames.push(CallFrame {
                    chunk_idx: fn_idx,
                    pc: 0,
                    locals_base: base,
                    upvalues: Box::default(),
                    closure_home: None,
                    source_slots: Box::default(),
                });
                return Ok(Step::Continue);
            }
            return Err(VmError::TypeMismatch("Mul"));
        }
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_div(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            state.stack.push(Value::Int(state.overflow_mode.div(x, y)?));
        }
        (Value::Float(x), Value::Float(y)) => {
            state.stack.push(Value::Float(x / y));
        }
        _ => return Err(VmError::TypeMismatch("Div")),
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_mod(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            state.stack.push(Value::Int(state.overflow_mode.rem(x, y)?));
        }
        (Value::Float(x), Value::Float(y)) => {
            state.stack.push(Value::Float(x % y));
        }
        _ => return Err(VmError::TypeMismatch("Mod")),
    }
    Ok(Step::Continue)
}

#[inline(never)]
fn h_neg(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match v {
        Value::Int(i) => {
            state
                .stack
                .push(Value::Int(state.overflow_mode.neg(i, "Neg")?));
        }
        Value::Float(f) => {
            state.stack.push(Value::Float(-f));
        }
        _ => return Err(VmError::TypeMismatch("Neg")),
    }
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
    if !state.try_stack.is_empty() && !func.fails.is_empty() {
        let variant = func.fails[0].clone();
        let arity = func.arity as usize;
        for _ in 0..arity {
            state.stack.pop();
        }
        return Ok(Step::CatchDispatch(variant));
    }
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
        upvalues: Box::default(),
        closure_home: None,
        source_slots: Box::default(),
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_return_from_call(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let ret = state.stack.pop().unwrap_or(Value::Void);
    let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
    if state.frames.is_empty() {
        return Ok(Step::Halt(ret));
    }
    let caller_base = state.frames.last().map_or(0, |f| f.locals_base);
    write_back_upvalues(&popped, caller_base, &mut state.locals);
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
        Value::Float(f) => f == 0.0,
        Value::String(ref s) => s.is_empty(),
        _ => false,
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
        Value::Float(f) => f != 0.0,
        Value::String(ref s) => !s.is_empty(),
        _ => true,
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
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    state.stack.push(Value::Bool(vm_values_eq_checked(&a, &b)?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_neq(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    state
        .stack
        .push(Value::Bool(!vm_values_eq_checked(&a, &b)?));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_lt(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let result = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x < y,
        (Value::Float(x), Value::Float(y)) => x < y,
        (Value::String(ref x), Value::String(ref y)) => x < y,
        (Value::Char(x), Value::Char(y)) => x < y,
        _ => return Err(VmError::TypeMismatch("Lt")),
    };
    state.stack.push(Value::Bool(result));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_le(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let result = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x <= y,
        (Value::Float(x), Value::Float(y)) => x <= y,
        (Value::String(ref x), Value::String(ref y)) => x <= y,
        (Value::Char(x), Value::Char(y)) => x <= y,
        _ => return Err(VmError::TypeMismatch("Le")),
    };
    state.stack.push(Value::Bool(result));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_gt(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let result = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x > y,
        (Value::Float(x), Value::Float(y)) => x > y,
        (Value::String(ref x), Value::String(ref y)) => x > y,
        (Value::Char(x), Value::Char(y)) => x > y,
        _ => return Err(VmError::TypeMismatch("Gt")),
    };
    state.stack.push(Value::Bool(result));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_ge(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let b = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let a = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let result = match (a, b) {
        (Value::Int(x), Value::Int(y)) => x >= y,
        (Value::Float(x), Value::Float(y)) => x >= y,
        (Value::String(ref x), Value::String(ref y)) => x >= y,
        (Value::Char(x), Value::Char(y)) => x >= y,
        _ => return Err(VmError::TypeMismatch("Ge")),
    };
    state.stack.push(Value::Bool(result));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_not(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let negated = match v {
        Value::Bool(b) => !b,
        Value::Int(i) => i == 0,
        Value::Float(f) => f == 0.0,
        Value::String(ref s) => s.is_empty(),
        _ => false,
    };
    state.stack.push(Value::Bool(negated));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_return(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    Ok(Step::Halt(state.stack.pop().unwrap_or(Value::Void)))
}

#[inline(never)]
fn h_make_closure(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::MakeClosure {
        fn_idx,
        upvalue_count,
    } = op
    else {
        unreachable!()
    };
    if state.stack.len() < upvalue_count as usize {
        return Err(VmError::EmptyStack);
    }
    let split = state.stack.len() - upvalue_count as usize;
    let captured: Box<[Value]> = state
        .stack
        .drain(split..)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let src = state
        .program
        .functions
        .get(fn_idx as usize)
        .map(|f| f.upvalue_source_slots.clone())
        .unwrap_or_default();
    state.stack.push(Value::Closure {
        fn_idx,
        upvalues: captured,
        source_slots: src,
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_load_upvalue(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::LoadUpvalue(idx) = op else {
        unreachable!()
    };
    let frame_idx = state.frame_idx();
    let v = state.frames[frame_idx]
        .upvalues
        .get(idx as usize)
        .ok_or(VmError::UpvalueOutOfBounds(idx))?
        .clone();
    state.stack.push(v);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_call_closure(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::CallClosure { arity, source_slot } = op else {
        unreachable!()
    };
    if state.stack.len() < arity as usize + 1 {
        return Err(VmError::EmptyStack);
    }
    if state.frames.len() >= MAX_CALL_DEPTH {
        return Err(VmError::CallStackOverflow);
    }
    let caller_base = state.frames.last().map_or(0, |f| f.locals_base);
    let home = if source_slot != u16::MAX {
        Some(caller_base + source_slot as usize)
    } else {
        None
    };
    let split = state.stack.len() - arity as usize;
    let args: Vec<Value> = state.stack.drain(split..).collect();
    let closure = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let (fn_idx, captured, src_slots) = match closure {
        Value::Closure {
            fn_idx,
            upvalues,
            source_slots,
        } => (fn_idx, upvalues, source_slots),
        // RES-3915: calling a first-class enum constructor value
        // (`Color::Rgb` passed to a HOF, then invoked as `f(x)`) builds the
        // corresponding EnumVariant, mirroring the interpreter's
        // `apply_function` → `apply_constructor` path.
        Value::EnumConstructor {
            type_name,
            variant,
            arity: ctor_arity,
        } => {
            let result =
                crate::enum_ctors::apply_constructor(&type_name, &variant, ctor_arity, args)
                    .map_err(|_| {
                        VmError::TypeMismatch("CallClosure: enum constructor arity mismatch")
                    })?;
            state.stack.push(result);
            return Ok(Step::Continue);
        }
        _ => return Err(VmError::TypeMismatch("CallClosure: expected Closure")),
    };
    let func = state
        .program
        .functions
        .get(fn_idx as usize)
        .ok_or(VmError::FunctionOutOfBounds(fn_idx))?;
    let base = state.locals.len();
    state
        .locals
        .resize(base + func.local_count as usize, Value::Void);
    for (i, v) in args.into_iter().enumerate() {
        state.locals[base + i] = v;
    }
    state.frames.push(CallFrame {
        chunk_idx: fn_idx as usize,
        pc: 0,
        locals_base: base,
        upvalues: captured,
        closure_home: home,
        source_slots: src_slots,
    });
    Ok(Step::Continue)
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
    let target = state.stack.pop().ok_or(VmError::EmptyStack)?;
    if let Value::Map(m) = target {
        let mk = crate::MapKey::from_value(&idx_val).map_err(VmError::BuiltinCallFailed)?;
        let v = m.get(&mk).cloned().unwrap_or(Value::Void);
        state.stack.push(v);
        return Ok(Step::Continue);
    }
    let Value::Int(idx) = idx_val else {
        return Err(VmError::TypeMismatch("LoadIndex (non-int index)"));
    };
    match target {
        Value::Array(mut items) => {
            let len = items.len() as i64;
            let resolved = if idx < 0 { idx + len } else { idx };
            if resolved < 0 || resolved >= len {
                return Err(VmError::ArrayIndexOutOfBounds {
                    index: idx,
                    len: items.len(),
                });
            }
            state.stack.push(items.swap_remove(resolved as usize));
        }
        Value::Tuple(mut items) => {
            if idx < 0 || (idx as usize) >= items.len() {
                return Err(VmError::ArrayIndexOutOfBounds {
                    index: idx,
                    len: items.len(),
                });
            }
            state.stack.push(items.swap_remove(idx as usize));
        }
        Value::String(s) => {
            let len = s.chars().count();
            let len_i = len as i64;
            let resolved = if idx < 0 { idx + len_i } else { idx };
            if resolved < 0 || resolved >= len_i {
                return Err(VmError::ArrayIndexOutOfBounds { index: idx, len });
            }
            let ch = s
                .chars()
                .nth(resolved as usize)
                .expect("index already bounds-checked");
            // RES-3889: yield `Value::Char` to match the interpreter — see the
            // match-loop `Op::LoadIndex` arm for the parity rationale.
            state.stack.push(Value::Char(ch));
        }
        _ => return Err(VmError::TypeMismatch("LoadIndex (non-array target)")),
    }
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
    let Value::Array(mut items) = arr_val else {
        return Err(VmError::TypeMismatch(
            "LoadIndexUnchecked (non-array target)",
        ));
    };
    let len = items.len() as i64;
    let resolved = if idx < 0 { idx + len } else { idx };
    if resolved < 0 || resolved >= len {
        return Err(VmError::ArrayIndexOutOfBounds {
            index: idx,
            len: items.len(),
        });
    }
    state.stack.push(items.swap_remove(resolved as usize));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_store_index(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let idx_val = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let container = state.stack.pop().ok_or(VmError::EmptyStack)?;
    if let Value::Map(mut m) = container {
        let mk = crate::MapKey::from_value(&idx_val).map_err(VmError::BuiltinCallFailed)?;
        m.insert(mk, v);
        state.stack.push(Value::Map(m));
        return Ok(Step::Continue);
    }
    let Value::Int(idx) = idx_val else {
        return Err(VmError::TypeMismatch("StoreIndex (non-int index)"));
    };
    let Value::Array(mut items) = container else {
        return Err(VmError::TypeMismatch("StoreIndex (non-array target)"));
    };
    let len = items.len() as i64;
    let resolved = if idx < 0 { idx + len } else { idx };
    if resolved < 0 || resolved >= len {
        return Err(VmError::ArrayIndexOutOfBounds {
            index: idx,
            len: items.len(),
        });
    }
    items[resolved as usize] = v;
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
    // RES-1459: drain instead of pop+reverse — see the match-dispatch
    // CallForeign arm in run_inner for the justification.
    let split_at = state.stack.len() - arity;
    let args: Vec<crate::Value> = state.stack.drain(split_at..).collect();
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
    // RES-1421: hold the builtin name as `&str` borrowed from the
    // chunk's constant pool — same justification as the match-dispatch
    // arm in `run_inner`. Saves a `String::clone` per builtin
    // dispatch in the direct-threaded VM path.
    let chunk = state.current_chunk();
    let name_val = chunk
        .constants
        .get(name_const as usize)
        .ok_or(VmError::ConstantOutOfBounds(name_const))?;
    let name: &str = match name_val {
        Value::String(s) => s.as_str(),
        _ => return Err(VmError::TypeMismatch("CallBuiltin (non-string name)")),
    };
    let n = arity as usize;
    if state.stack.len() < n {
        return Err(VmError::EmptyStack);
    }
    // RES-1459: drain instead of pop+reverse — same as CallForeign /
    // match-dispatch CallBuiltin above.
    let split_at = state.stack.len() - n;
    let args: Vec<Value> = state.stack.drain(split_at..).collect();
    // RES-3994: `to_string(struct_val)` Display-impl dispatch — see
    // the `run_inner` `Op::CallBuiltin` arm for the full rationale;
    // this is the direct-threaded-dispatch twin, kept byte-identical.
    if let Some(fn_idx) = vm_display_fmt_fn_idx(state.program, name, &args) {
        if state.frames.len() >= MAX_CALL_DEPTH {
            return Err(VmError::CallStackOverflow);
        }
        let func = &state.program.functions[fn_idx];
        let base = state.locals.len();
        state
            .locals
            .resize(base + func.local_count as usize, Value::Void);
        state.locals[base] = args.into_iter().next().expect("checked len == 1 above");
        state.frames.push(CallFrame {
            chunk_idx: fn_idx,
            pc: 0,
            locals_base: base,
            upvalues: Box::default(),
            closure_home: None,
            source_slots: Box::default(),
        });
        return Ok(Step::Continue);
    }
    // Try the non-namespaced builtin table first, then fall back to
    // stdlib qualified names (e.g. "math::sqrt") so `use std::math;`
    // works under --vm.
    let result = if let Some(func) = crate::lookup_builtin(name) {
        func(&args).map_err(VmError::BuiltinCallFailed)?
    } else if let Some(stdlib_result) = crate::stdlib::call_by_qualified_name(name, &args) {
        stdlib_result.map_err(VmError::BuiltinCallFailed)?
    } else {
        return Err(VmError::UnknownBuiltin(name.to_string()));
    };
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
    // RES-1433: borrow the field name from the constant pool. Same
    // justification as the match-dispatch GetField arm above —
    // owned String was only used in the (rare) UnknownField error.
    let chunk = state.current_chunk();
    let field = constant_as_str(chunk, name_const, "GetField (field name)")?;
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let val = vm_get_field_value(v, field)?;
    state.stack.push(val);
    Ok(Step::Continue)
}

/// RES-3918: shared `GetField` resolution for both dispatch engines.
/// Reads a field from a `Struct` (by name) or an `EnumVariant` payload —
/// the latter reached when `match` lowers payload binding to `GetField`
/// against the scrutinee (`Named` field name for named payloads, the
/// stringified tuple index `"0"`/`"1"` for tuple payloads). Before this,
/// the handler only matched `Value::Struct`, so every payload-extracting
/// enum match crashed with `GetField (non-struct target)`.
fn vm_get_field_value(v: Value, field: &str) -> Result<Value, VmError> {
    match v {
        Value::Struct { name, fields } => fields
            .iter()
            .find(|(k, _)| k.as_str() == field)
            .map(|(_, fv)| fv.clone())
            .ok_or(VmError::UnknownField {
                struct_name: name,
                field: field.to_string(),
            }),
        Value::EnumVariant {
            type_name,
            variant,
            payload,
        } => {
            let qualified = || format!("{type_name}::{variant}");
            match payload {
                crate::EnumValuePayload::Named(fields) => fields
                    .iter()
                    .find(|(k, _)| k.as_str() == field)
                    .map(|(_, fv)| fv.clone())
                    .ok_or_else(|| VmError::UnknownField {
                        struct_name: qualified(),
                        field: field.to_string(),
                    }),
                crate::EnumValuePayload::Tuple(vals) => field
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| vals.get(i).cloned())
                    .ok_or_else(|| VmError::UnknownField {
                        struct_name: qualified(),
                        field: field.to_string(),
                    }),
                crate::EnumValuePayload::None => Err(VmError::UnknownField {
                    struct_name: qualified(),
                    field: field.to_string(),
                }),
            }
        }
        _ => Err(VmError::TypeMismatch("GetField (non-struct target)")),
    }
}

#[inline(never)]
fn h_set_field(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::SetField { name_const } = op else {
        unreachable!()
    };
    // RES-1433: borrow the field name from the constant pool. Same
    // justification as the GetField handler above.
    let chunk = state.current_chunk();
    let field = constant_as_str(chunk, name_const, "SetField (field name)")?;
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let tgt = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let Value::Struct {
        name: sname,
        mut fields,
    } = tgt
    else {
        return Err(VmError::TypeMismatch("SetField (non-struct target)"));
    };
    let slot = fields.iter_mut().find(|(k, _)| k.as_str() == field);
    match slot {
        Some((_, existing)) => *existing = v,
        None => {
            return Err(VmError::UnknownField {
                struct_name: sname,
                field: field.to_string(),
            });
        }
    }
    state.stack.push(Value::Struct {
        name: sname,
        fields,
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_band(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Band")?;
    state.stack.push(Value::Int(a & b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_bor(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Bor")?;
    state.stack.push(Value::Int(a | b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_bxor(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Bxor")?;
    state.stack.push(Value::Int(a ^ b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_shl(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Shl")?;
    if !(0..64).contains(&b) {
        return Err(VmError::ShiftOutOfRange(b));
    }
    state.stack.push(Value::Int(a << b));
    Ok(Step::Continue)
}

#[inline(never)]
fn h_shr(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let (a, b) = pop_two_ints(&mut state.stack, "Shr")?;
    if !(0..64).contains(&b) {
        return Err(VmError::ShiftOutOfRange(b));
    }
    state.stack.push(Value::Int(a >> b));
    Ok(Step::Continue)
}

fn h_assert_fail(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let msg = match state.stack.pop().ok_or(VmError::EmptyStack)? {
        Value::String(s) => s,
        other => format!("assertion failed: {}", other),
    };
    Err(VmError::AssertionFailed(msg))
}

fn h_assume_fail(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let msg = match state.stack.pop().ok_or(VmError::EmptyStack)? {
        Value::String(s) => s,
        other => format!("assumption failed: {}", other),
    };
    Err(VmError::AssumeViolated(msg))
}

fn h_assert_bool(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    if !matches!(v, Value::Bool(_)) {
        return Err(VmError::TypeMismatch("&& / || operand"));
    }
    state.stack.push(v);
    Ok(Step::Continue)
}

/// RES-3997: discard TOS. See `Op::Pop` doc comment in `bytecode.rs`.
#[inline(never)]
fn h_pop(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    state.stack.pop().ok_or(VmError::EmptyStack)?;
    Ok(Step::Continue)
}

#[inline(never)]
fn h_make_tuple(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::MakeTuple { len } = op else {
        return Err(VmError::Unsupported("h_make_tuple: wrong op"));
    };
    let n = len as usize;
    if state.stack.len() < n {
        return Err(VmError::EmptyStack);
    }
    let split_at = state.stack.len() - n;
    let items: Vec<Value> = state.stack.drain(split_at..).collect();
    state.stack.push(Value::Tuple(items));
    Ok(Step::Continue)
}

// RES-375/RES-363: try-unwrap handler for the direct-threaded path.
// Mirrors the run_inner Op::TryUnwrap arm exactly.
#[inline(never)]
fn h_try_unwrap(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    if !matches!(op, Op::TryUnwrap) {
        return Err(VmError::Unsupported("h_try_unwrap: wrong op"));
    }
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    match v {
        Value::Result { ok: true, payload } => {
            state.stack.push(*payload);
            Ok(Step::Continue)
        }
        Value::Option(Some(inner)) => {
            state.stack.push(*inner);
            Ok(Step::Continue)
        }
        Value::Result { ok: false, payload } => {
            let ret = Value::Result { ok: false, payload };
            let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
            if state.frames.is_empty() {
                return Ok(Step::Halt(ret));
            }
            state.locals.truncate(popped.locals_base);
            state.stack.push(ret);
            Ok(Step::Continue)
        }
        Value::Option(None) => {
            let popped = state.frames.pop().ok_or(VmError::CallStackUnderflow)?;
            if state.frames.is_empty() {
                return Ok(Step::Halt(Value::Option(None)));
            }
            state.locals.truncate(popped.locals_base);
            state.stack.push(Value::Option(None));
            Ok(Step::Continue)
        }
        _ => Err(VmError::TypeMismatch(
            "TryUnwrap: expected Result or Option",
        )),
    }
}

fn h_iter_prepare(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    state.stack.push(iter_prepare_value(v)?);
    Ok(Step::Continue)
}

/// `Op::IterPrepare` normalizes a `for`-loop iterable into a shape the
/// rest of `compile_for_in`'s sequential `LoadIndex` loop can walk
/// uniformly: `Array`/`String` pass through unchanged, `Map` becomes
/// its sorted-keys array, and (RES-4000) `Range` is materialized into
/// an `Array` of `Value::Int` via the same `crate::ranges::iterate_range`
/// driver the tree-walker's non-literal-range `for` path uses (`lib.rs`
/// `eval_for_in_in_scope`). This keeps `Node::Range` itself lowering to
/// a first-class `Value::Range` (see `compiler.rs`) for `type_of`/`len`/
/// `contains`/`to_string` parity, while iteration still gets a concrete
/// array to index into.
fn iter_prepare_value(v: Value) -> Result<Value, VmError> {
    match v {
        Value::Array(_) | Value::String(_) => Ok(v),
        Value::Map(m) => {
            let mut keys: Vec<&crate::MapKey> = m.keys().collect();
            keys.sort_unstable_by(|a, b| crate::map_entries_merge::cmp_map_keys(a, b));
            let arr: Vec<Value> = keys.into_iter().map(|k| k.to_value()).collect();
            Ok(Value::Array(arr))
        }
        Value::Range {
            start,
            end,
            inclusive,
        } => {
            let arr: Vec<Value> = crate::ranges::iterate_range(start, end, inclusive)
                .map(Value::Int)
                .collect();
            Ok(Value::Array(arr))
        }
        _ => Err(VmError::TypeMismatch(
            "IterPrepare: expected Array, String, Map, or Range",
        )),
    }
}

#[inline(never)]
fn h_load_global(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::LoadGlobal(idx) = op else {
        unreachable!()
    };
    let abs = idx as usize;
    let v = state
        .locals
        .get(abs)
        .ok_or(VmError::LocalOutOfBounds(idx))?
        .clone();
    state.stack.push(v);
    Ok(Step::Continue)
}

#[inline(never)]
fn h_store_global(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::StoreGlobal(idx) = op else {
        unreachable!()
    };
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let abs = idx as usize;
    if state.locals.len() <= abs {
        state.locals.resize(abs + 1, Value::Void);
    }
    state.locals[abs] = v;
    Ok(Step::Continue)
}

#[inline(never)]
fn h_store_upvalue(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::StoreUpvalue {
        upvalue_idx,
        local_slot,
    } = op
    else {
        unreachable!()
    };
    let v = state.stack.pop().ok_or(VmError::EmptyStack)?;
    let frame_idx = state.frame_idx();
    let frame = &mut state.frames[frame_idx];
    let abs = frame.locals_base + local_slot as usize;
    if state.locals.len() <= abs {
        state.locals.resize(abs + 1, Value::Void);
    }
    state.locals[abs] = v.clone();
    if let Some(uv) = frame.upvalues.get_mut(upvalue_idx as usize) {
        *uv = v;
    }
    Ok(Step::Continue)
}

fn h_call_method(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::CallMethod {
        method_const,
        arity,
    } = op
    else {
        unreachable!()
    };
    let chunk = state.current_chunk();
    let method = match &chunk.constants[method_const as usize] {
        Value::String(s) => s.clone(),
        _ => {
            return Err(VmError::TypeMismatch(
                "CallMethod: bad method name constant",
            ));
        }
    };
    let arity = arity as usize;
    if state.stack.len() < arity + 1 {
        return Err(VmError::EmptyStack);
    }
    if state.frames.len() >= MAX_CALL_DEPTH {
        return Err(VmError::CallStackOverflow);
    }
    let split = state.stack.len() - arity;
    let args: Vec<Value> = state.stack.drain(split..).collect();
    let receiver = state.stack.pop().ok_or(VmError::EmptyStack)?;
    // RES-3994: primitive-`impl` receivers (`impl int { ... }`, `impl
    // float`, `impl string`, `impl bool` — RES-2553) mangle the same
    // way struct/enum methods do (`int$abs`).
    let mangled_prefix = match &receiver {
        Value::Struct { name, .. } => Some(name.clone()),
        Value::EnumVariant { type_name, .. } => Some(type_name.clone()),
        other => vm_primitive_impl_type_name(other).map(str::to_string),
    };
    let Some(prefix) = mangled_prefix else {
        // RES-3904: not a struct/enum/primitive-impl receiver — fall
        // back to the built-in container method sugar (String/Array/
        // Map/Set), which the compiler emits `CallMethod` for
        // identically since it has no static type info.
        let result = vm_call_builtin_method(receiver, &method, args)?;
        state.stack.push(result);
        return Ok(Step::Continue);
    };
    let mangled = format!("{}${}", prefix, method);
    let Some(fn_idx) = state
        .program
        .functions
        .iter()
        .position(|f| f.name == mangled)
    else {
        // RES-3994: no matching `impl` method. Struct/enum receivers
        // keep the existing hard error; primitive scalars fall back
        // to the generic built-in method dispatch instead, mirroring
        // the interpreter falling through past its primitive-impl
        // check to the array-functional / generic builtin dispatch.
        if matches!(&receiver, Value::Struct { .. } | Value::EnumVariant { .. }) {
            return Err(VmError::TypeMismatch("CallMethod: method not found"));
        }
        let result = vm_call_builtin_method(receiver, &method, args)?;
        state.stack.push(result);
        return Ok(Step::Continue);
    };
    let func = &state.program.functions[fn_idx];
    let base = state.locals.len();
    state
        .locals
        .resize(base + func.local_count as usize, Value::Void);
    state.locals[base] = receiver;
    for (i, v) in args.into_iter().enumerate() {
        state.locals[base + 1 + i] = v;
    }
    state.frames.push(CallFrame {
        chunk_idx: fn_idx,
        pc: 0,
        locals_base: base,
        upvalues: Box::default(),
        closure_home: None,
        source_slots: Box::default(),
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_enter_try(state: &mut VmState<'_>, op: Op) -> Result<Step, VmError> {
    let Op::EnterTry(handler_idx) = op else {
        unreachable!()
    };
    let frame_idx = state.frame_idx();
    state.try_stack.push(TryFrame {
        handler_table_idx: handler_idx,
        chunk_idx: state.frames[frame_idx].chunk_idx,
        call_depth: state.frames.len(),
        stack_depth: state.stack.len(),
    });
    Ok(Step::Continue)
}

#[inline(never)]
fn h_exit_try(state: &mut VmState<'_>, _op: Op) -> Result<Step, VmError> {
    state.try_stack.pop();
    Ok(Step::Continue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MapKey;
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

    // --- RES-3995: `live { ... }` retry loop under `--vm` ---

    #[test]
    fn live_block_retries_until_success_returns_value() {
        // Mirrors examples/live_retry_log.rz: fails twice, succeeds on
        // the third attempt, and the block's own value flows out.
        let src = "\
            static let fails_left = 2;\n\
            fn maybe_fail() {\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced fail\");\n\
                }\n\
                return 42;\n\
            }\n\
            fn main(int _d) {\n\
                live {\n\
                    maybe_fail();\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        assert!(
            compile_run(src).is_ok(),
            "expected the block to eventually succeed"
        );
    }

    #[test]
    fn live_retries_reports_attempt_number_across_retries() {
        // `live_retries()` must read 0, 1, 2 across the three attempts
        // — same counter arithmetic as the tree-walker's `LiveRetryGuard`.
        let src = "\
            static let fails_left = 2;\n\
            static let seen = 0;\n\
            fn main(int _d) {\n\
                live {\n\
                    seen = live_retries();\n\
                    if fails_left > 0 {\n\
                        fails_left = fails_left - 1;\n\
                        assert(false, \"forced fail\");\n\
                    }\n\
                }\n\
                return seen;\n\
            }\n\
            main(0);\n\
        ";
        assert_int(compile_run(src).unwrap(), 2);
    }

    #[test]
    fn live_retries_outside_live_block_is_clean_error() {
        let err = compile_run("live_retries();").unwrap_err();
        let display = err.to_string();
        assert!(
            display.contains("live_retries() called outside a live block"),
            "unexpected error text: {}",
            display
        );
    }

    #[test]
    fn live_block_invariant_failure_triggers_retry_and_rolls_back_state() {
        // `live invariant` treats a false invariant exactly like a body
        // error: retry with the pre-attempt state restored. `count` is
        // declared OUTSIDE the block (in the enclosing fn), so this
        // also exercises that the retry rollback covers the enclosing
        // scope's locals, not just the block's own — mirrors
        // examples/showcase_live_invariant.rz's rollback requirement.
        let src = "\
            fn main(int _d) {\n\
                let count = 0;\n\
                live invariant count <= 1 {\n\
                    count = count + 1;\n\
                    if live_retries() == 0 {\n\
                        count = count + 5;\n\
                    }\n\
                }\n\
                return count;\n\
            }\n\
            main(0);\n\
        ";
        // Attempt 0: count = 0+1+5 = 6, invariant (6 <= 1) fails, retry
        // (count rolls back to 0). Attempt 1: count = 0+1 = 1, holds.
        assert_int(compile_run(src).unwrap(), 1);
    }

    #[test]
    fn live_block_exhausts_retry_budget_and_propagates_error() {
        // Always fails — `retries(1)` caps the budget at exactly one
        // attempt, so the assertion error must propagate rather than
        // retry forever.
        let src = "\
            fn main(int _d) {\n\
                live retries(1) {\n\
                    assert(false, \"always fails\");\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let err = compile_run(src).unwrap_err();
        let display = err.to_string();
        assert!(
            display.contains("always fails"),
            "expected the underlying assertion text to propagate: {}",
            display
        );
    }

    #[test]
    fn live_block_early_return_from_body_exits_successfully() {
        // A `return` inside a live block's body (e.g.
        // examples/thermal_safety_cutoff.rz's `safe_read`) must return
        // from the enclosing function, not be treated as a failure.
        let src = "\
            fn safe_read(int nominal) -> int {\n\
                live invariant true {\n\
                    let reading = nominal;\n\
                    return reading;\n\
                }\n\
            }\n\
            fn main() {\n\
                return safe_read(500);\n\
            }\n\
            main();\n\
        ";
        assert_int(compile_run(src).unwrap(), 500);
    }

    #[test]
    fn live_block_retries_are_visible_via_live_total_retries() {
        // RES-141: `VM_LIVE_TOTAL_RETRIES` is a process-wide counter
        // shared across every test in this binary (mirrors the
        // tree-walker's own `LIVE_TOTAL_RETRIES` and its test's
        // before/after-delta pattern) — snapshot before running so
        // parallel test execution can't make this flaky.
        use std::sync::atomic::Ordering::Relaxed;
        let before = VM_LIVE_TOTAL_RETRIES.load(Relaxed);
        let src = "\
            static let fails_left = 2;\n\
            fn main(int _d) {\n\
                live {\n\
                    if fails_left > 0 {\n\
                        fails_left = fails_left - 1;\n\
                        assert(false, \"forced\");\n\
                    }\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        assert!(compile_run(src).is_ok());
        let after = VM_LIVE_TOTAL_RETRIES.load(Relaxed);
        // Lower-bound, not exact — other tests running in parallel on
        // the same process bump this same atomic, inflating the delta
        // (never deflating it below what this test's own workload
        // contributes).
        assert!(
            after - before >= 2,
            "expected at least 2 retries counted from this test's own workload, saw delta {}",
            after - before
        );
    }

    #[test]
    fn int_plus_string_coerces() {
        let p = const_program(
            &[Value::Int(1), Value::String("x".into())],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&p).unwrap() {
            Value::String(s) => assert_eq!(s, "1x"),
            other => panic!("expected String, got {:?}", other),
        }
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

    // ---------- bitwise op tests ----------

    #[test]
    fn bitwise_and() {
        assert_int(compile_run("15 & 10;").unwrap(), 10);
    }

    #[test]
    fn bitwise_or() {
        assert_int(compile_run("5 | 10;").unwrap(), 15);
    }

    #[test]
    fn bitwise_xor() {
        assert_int(compile_run("15 ^ 5;").unwrap(), 10);
    }

    #[test]
    fn bitwise_shift_left() {
        assert_int(compile_run("1 << 4;").unwrap(), 16);
    }

    #[test]
    fn bitwise_shift_right() {
        assert_int(compile_run("256 >> 3;").unwrap(), 32);
    }

    #[test]
    fn bitwise_ops_in_function() {
        let src = "fn mask(int x) -> int { return (x & 0xFF) | 0x100; } mask(255);";
        assert_int(compile_run(src).unwrap(), 0x1FF);
    }

    #[test]
    fn shl_out_of_range_is_error() {
        // Shift amount 64 is out of the valid range 0..63; both the
        // interpreter and the VM must return an error (not silently mask).
        let err = compile_run("1 << 64;").unwrap_err();
        assert!(
            matches!(err.kind(), VmError::ShiftOutOfRange(64)),
            "expected ShiftOutOfRange(64), got {:?}",
            err
        );
    }

    #[test]
    fn shr_negative_amount_is_error() {
        let err = compile_run("1 >> -1;").unwrap_err();
        assert!(
            matches!(err.kind(), VmError::ShiftOutOfRange(-1)),
            "expected ShiftOutOfRange(-1), got {:?}",
            err
        );
    }

    #[test]
    fn shl_boundary_63_is_valid() {
        // 1 << 63 is the minimum i64 value; within the valid range.
        assert_int(compile_run("1 << 63;").unwrap(), i64::MIN);
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
            upvalue_source_slots: Box::default(),
            fails: Box::default(),
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

    // RES-3997 test change: these `if`/comparison/logical probes below
    // used to write `if COND { 1; } else { 2; }` as a bare top-level
    // statement and rely on `compile_run` returning `1`/`2` — which only
    // worked because the *bug* this ticket fixes (a discarded
    // statement-expression's value not being popped off the VM stack)
    // let the branch body's `1;`/`2;` leak all the way up to the
    // program's implicit result. That was never a real language
    // feature (a bare `if {...} else {...}` statement has no value —
    // only `if`-in-*expression*-position does, e.g. `let r = if b {1}
    // else {2};`), so once the leak is fixed these correctly compile to
    // `Void` instead of the branch's discarded literal. Rewritten to use
    // the actual if-expression idiom (`let r = if COND {A} else {B}; r`)
    // so the trailing bare `r` — not the leak — carries the value,
    // while still exercising the exact same comparison/logical-op
    // machinery the tests are named for.
    #[test]
    fn if_true_picks_consequence() {
        assert_int(
            compile_run("let r = if true { 1 } else { 2 }; r").unwrap(),
            1,
        );
    }

    #[test]
    fn if_false_picks_alternative() {
        assert_int(
            compile_run("let r = if false { 1 } else { 2 }; r").unwrap(),
            2,
        );
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
        // a public Bool probe, but `let r = if 3 < 5 {1} else {0}; r`
        // tells us 1 iff Lt evaluated to true.
        assert_int(
            compile_run("let r = if 3 < 5 { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if 5 < 3 { 1 } else { 0 }; r").unwrap(),
            0,
        );
        assert_int(
            compile_run("let r = if 5 == 5 { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if 5 != 5 { 1 } else { 0 }; r").unwrap(),
            0,
        );
    }

    #[test]
    fn logical_and_short_circuits() {
        // `false && <anything>` evaluates to false without evaluating rhs.
        // We can't directly observe short-circuit without side effects,
        // but we can at least confirm the result shape matches for
        // both paths.
        assert_int(
            compile_run("let r = if true && true { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if true && false { 1 } else { 0 }; r").unwrap(),
            0,
        );
        assert_int(
            compile_run("let r = if false && true { 1 } else { 0 }; r").unwrap(),
            0,
        );
    }

    #[test]
    fn logical_or_short_circuits() {
        assert_int(
            compile_run("let r = if true || false { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if false || true { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if false || false { 1 } else { 0 }; r").unwrap(),
            0,
        );
    }

    #[test]
    fn not_negates_boolean() {
        assert_int(
            compile_run("let r = if !false { 1 } else { 0 }; r").unwrap(),
            1,
        );
        assert_int(
            compile_run("let r = if !true { 1 } else { 0 }; r").unwrap(),
            0,
        );
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

    // ---------- RES-169c: closure opcodes are fully implemented ----------

    #[test]
    fn res169c_make_closure_produces_closure_value() {
        // MakeClosure with 0 upvalues should push a Closure value onto
        // the stack. We add a dummy function at index 0 so FunctionOutOfBounds
        // doesn't fire when CallClosure tries to look it up.
        use crate::bytecode::Function;
        let mut main = Chunk::new();
        main.code.push(Op::MakeClosure {
            fn_idx: 0,
            upvalue_count: 0,
        });
        main.code.push(Op::Return);
        main.line_info.push(1);
        main.line_info.push(1);
        let mut body = Chunk::new();
        body.code.push(Op::Return);
        body.line_info.push(1);
        let p = Program {
            main,
            functions: vec![Function {
                name: "f".into(),
                arity: 0,
                local_count: 0,
                upvalue_source_slots: Box::default(),
                fails: Box::default(),
                chunk: body,
            }],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let v = run(&p).unwrap();
        match v {
            Value::Closure {
                fn_idx, upvalues, ..
            } => {
                assert_eq!(fn_idx, 0);
                assert_eq!(upvalues.len(), 0);
            }
            other => panic!("expected Closure, got {:?}", other),
        }
    }

    #[test]
    fn res169c_load_upvalue_out_of_bounds_errors() {
        // LoadUpvalue(5) on a frame with 0 upvalues → UpvalueOutOfBounds
        let p = const_program(&[], &[Op::LoadUpvalue(5)]);
        let err = run(&p).unwrap_err();
        match err.kind() {
            VmError::UpvalueOutOfBounds(idx) => assert_eq!(*idx, 5),
            other => panic!("expected UpvalueOutOfBounds(5), got {:?}", other),
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
            source_slots: Box::default(),
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

    // ---------- RES-169d: FunctionLiteral + indirect call (closures from source) ----------

    #[test]
    fn res169d_function_literal_basic_call() {
        // let double = fn(int x) { return x * 2; }; double(5);
        let src = "let double = fn(int x) { return x * 2; }; double(5);";
        assert_int(compile_run(src).unwrap(), 10);
    }

    #[test]
    fn res169d_function_literal_captures_outer_variable() {
        // let n = 10; let adder = fn(int x) { return x + n; }; adder(5);
        let src = "let n = 10; let adder = fn(int x) { return x + n; }; adder(5);";
        assert_int(compile_run(src).unwrap(), 15);
    }

    #[test]
    fn res169d_function_literal_no_args() {
        // let get_42 = fn() { return 42; }; get_42();
        let src = "let get_42 = fn() { return 42; }; get_42();";
        assert_int(compile_run(src).unwrap(), 42);
    }

    #[test]
    fn res169d_function_literal_capture_multiple_vars() {
        // let a = 3; let b = 4; let sum = fn() { return a + b; }; sum();
        let src = "let a = 3; let b = 4; let sum_fn = fn() { return a + b; }; sum_fn();";
        assert_int(compile_run(src).unwrap(), 7);
    }

    #[test]
    fn res169d_closure_immediate_call() {
        // Immediately-called function literal: fn(int x) { return x * 3; }(7)
        // is not yet parser-supported, but we can inline it via let.
        let src = "let mul3 = fn(int x) { return x * 3; }; mul3(7);";
        assert_int(compile_run(src).unwrap(), 21);
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
    fn res171a_load_index_negative_wraps() {
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
        assert_int(run(&p).unwrap(), 2);
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
    fn res334b_load_index_on_string_returns_char() {
        // RES-334b: `s[i]` on a string returns the i-th character.
        // RES-3889: as a `Value::Char`, matching the tree-walker interpreter —
        // previously the VM returned a single-char `Value::String`, which
        // diverged from the interpreter for `s[i] == 'c'`.
        let v = compile_run(r#"let s = "hello"; return s[1];"#).unwrap();
        match v {
            crate::Value::Char(c) => assert_eq!(c, 'e'),
            other => panic!("expected Char, got {:?}", other),
        }
    }

    #[test]
    fn res334b_load_index_on_string_oob_errors() {
        let err = compile_run(r#"let s = "hi"; return s[5];"#).unwrap_err();
        assert!(
            err.to_string().contains("out of bounds"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn res171c_nested_index_assignment_compiles_and_runs() {
        // RES-171c: a[i][j] = v now compiles and runs correctly.
        // After `a[0][1] = 99`, a[0] should be [1, 99].
        let v = compile_run("let a = [[1,2],[3,4]]; a[0][1] = 99; return a[0][1];").unwrap();
        assert_int(v, 99);
    }

    #[test]
    fn res171c_nested_index_assignment_three_levels() {
        // a[0][0][0] = 7 updates the innermost element.
        let v = compile_run("let a = [[[1,2],[3,4]]]; a[0][0][0] = 7; return a[0][0][0];").unwrap();
        assert_int(v, 7);
    }

    #[test]
    fn res171c_nested_index_assignment_preserves_other_elements() {
        // Updating a[1][0] must not disturb a[0].
        let v = compile_run("let a = [[10,20],[30,40]]; a[1][0] = 99; return a[0][0];").unwrap();
        assert_int(v, 10);
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

    // ============================================================
    // RES-3891: cross-type `==` / `!=` is a runtime type mismatch on
    // the VM, matching the tree-walking interpreter (which raises
    // "Type mismatch" from `eval_infix`). Before this fix the VM's
    // `vm_values_eq` catch-all reported every cross-kind pair as
    // unequal, so identical source diverged between backends.
    // ============================================================

    fn assert_both_type_mismatch(src: &str) {
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        let m = run_with(&prog, OverflowMode::Wrap, Dispatch::Match).unwrap_err();
        let d = run_with(&prog, OverflowMode::Wrap, Dispatch::Direct).unwrap_err();
        assert_eq!(m.kind(), d.kind(), "dispatch engines disagree on {src:?}");
        assert!(
            matches!(m.kind(), VmError::TypeMismatch(_)),
            "expected TypeMismatch for {src:?}, got {:?}",
            m.kind()
        );
    }

    #[test]
    fn res3891_char_eq_int_is_type_mismatch() {
        assert_both_type_mismatch("let s = \"ab\"; s[0] == 5;");
    }

    #[test]
    fn res3891_char_neq_int_is_type_mismatch() {
        assert_both_type_mismatch("let s = \"ab\"; s[0] != 5;");
    }

    #[test]
    fn res3891_int_eq_string_is_type_mismatch() {
        assert_both_type_mismatch("1 == \"x\";");
    }

    #[test]
    fn res3891_int_eq_bool_is_type_mismatch() {
        assert_both_type_mismatch("1 == true;");
    }

    #[test]
    fn res3891_int_eq_float_is_type_mismatch() {
        assert_both_type_mismatch("1 == 1.0;");
    }

    #[test]
    fn res3891_same_type_equality_still_agrees() {
        // The fix must not disturb legal comparisons: same-kind scalars
        // and structural compound equality behave identically on both
        // dispatch engines and both truth values.
        assert_both_eq("1 == 1;");
        assert_both_eq("1 == 2;");
        assert_both_eq("\"a\" == \"a\";");
        assert_both_eq("\"a\" != \"b\";");
        assert_both_eq("true == true;");
        assert_both_eq("let s = \"ab\"; s[0] == s[0];");
        assert_both_eq("let a = [1, 2]; let b = [1, 2]; a == b;");
        assert_both_eq("let a = [1, 2]; let b = [1, 3]; a == b;");
    }

    #[test]
    fn res3891_nested_cross_type_stays_total() {
        // Cross-type *elements nested inside* compounds must stay total
        // (compare unequal, not error) — only the outermost comparison is
        // checked. Arrays of differing element kinds are unequal, not a
        // type mismatch.
        let src = "let a = [1, 2]; let b = [1, \"x\"]; a == b;";
        let (m, d) = run_both(src);
        assert_eq!(value_repr(&m.unwrap()), value_repr(&d.unwrap()));
    }

    // ============================================================
    // RES-3894: `&&` / `||` require bool operands on the VM, matching
    // the interpreter (`Logical '&&' requires bool operands`). Before
    // this fix the VM desugared them through the truthiness-coercing
    // `JumpIfFalse` / `Not`, so `5 && true` silently ran instead of
    // erroring. An `Op::AssertBool` after each evaluated operand closes
    // the gap without touching `if` / `while` / `!` coercion.
    // ============================================================

    #[test]
    fn res3894_and_non_bool_left_is_type_mismatch() {
        assert_both_type_mismatch("5 && true;");
    }

    #[test]
    fn res3894_and_non_bool_right_is_type_mismatch() {
        assert_both_type_mismatch("true && 5;");
    }

    #[test]
    fn res3894_and_zero_left_is_type_mismatch() {
        // 0 is falsy under coercion but not a bool — must still error, not
        // short-circuit to false.
        assert_both_type_mismatch("0 && true;");
    }

    #[test]
    fn res3894_and_string_left_is_type_mismatch() {
        assert_both_type_mismatch("\"a\" && true;");
    }

    #[test]
    fn res3894_or_non_bool_left_is_type_mismatch() {
        assert_both_type_mismatch("5 || false;");
    }

    #[test]
    fn res3894_or_non_bool_right_is_type_mismatch() {
        assert_both_type_mismatch("false || 5;");
    }

    #[test]
    fn res3894_or_zero_left_is_type_mismatch() {
        assert_both_type_mismatch("0 || false;");
    }

    #[test]
    fn res3894_bool_operands_still_agree() {
        // Legal all-bool logical expressions are unaffected on both dispatch
        // engines and both truth values.
        assert_both_eq("true && false;");
        assert_both_eq("true && true;");
        assert_both_eq("false || true;");
        assert_both_eq("false || false;");
        assert_both_eq("(3 < 5) && (2 > 1);");
        assert_both_eq("let a = true; let b = false; a && b || true;");
    }

    #[test]
    fn res3894_short_circuit_skips_asserting_dead_operand() {
        // `&&` must not evaluate (or assert) the right operand when the left
        // is `false`; `||` must not when the left is `true`. A non-bool /
        // trapping right operand in the dead branch stays untouched.
        assert_both_eq("false && (1 / 0 > 0);");
        assert_both_eq("true || (1 / 0 > 0);");
    }

    #[test]
    fn res3894_non_bool_if_while_not_still_coerce() {
        // The fix is scoped to `&&` / `||`. `if` / `while` / unary `!` keep
        // their existing truthiness coercion, which already agrees across
        // backends — guard against an over-broad regression.
        assert_both_eq("if 5 { 1; } else { 0; }");
        assert_both_eq("if 0 { 1; } else { 0; }");
        assert_both_eq("let n = 3; while n { n = n - 1; } n;");
        assert_both_eq("!5;");
        assert_both_eq("!0;");
    }

    // ============================================================
    // RES-3896: `Array + Array` concatenates on the VM, matching the
    // interpreter. Before this fix `Op::Add` had no arm for two arrays
    // in either dispatch engine, so `[1,2] + [3,4]` raised
    // `VmError::TypeMismatch("Add")` on the VM while the interpreter's
    // `eval_infix_expression` already special-cased it and returned the
    // concatenated array.
    // ============================================================

    #[test]
    fn res3896_array_plus_array_concatenates() {
        let (m, d) = run_both("[1, 2] + [3, 4];");
        let m = m.unwrap();
        let d = d.unwrap();
        assert_eq!(value_repr(&m), value_repr(&d));
        match m {
            Value::Array(v) => assert_eq!(v.len(), 4, "expected 4-element concatenated array"),
            other => panic!("expected Array, got {other:?}"),
        }
    }

    #[test]
    fn res3896_empty_array_plus_array_is_identity() {
        assert_both_eq("let a: [int32] = []; let b = [1, 2]; a + b;");
        assert_both_eq("let a = [1, 2]; let b: [int32] = []; a + b;");
    }

    #[test]
    fn res3896_array_plus_non_array_is_still_type_mismatch() {
        assert_both_type_mismatch("[1, 2] + 5;");
        assert_both_type_mismatch("5 + [1, 2];");
    }

    #[test]
    fn res3896_array_concat_agrees_across_dispatch_engines() {
        assert_both_eq("[1, 2] + [3, 4];");
        assert_both_eq("let a = [\"x\", \"y\"]; let b = [\"z\"]; a + b;");
    }

    // ============================================================
    // RES-3904: `Op::CallMethod` on a built-in container receiver
    // (String/Array/Map/Set) must dispatch through the same builtin
    // the interpreter uses, matching it, instead of raising
    // `TypeMismatch("CallMethod: receiver is not a struct or enum")`.
    // Before this fix EVERY dot-call method on these types crashed
    // the VM — the opcode only had an arm for `Value::Struct`/
    // `Value::EnumVariant` receivers.
    // ============================================================

    #[test]
    fn res3904_string_method_call_dispatches_to_builtin() {
        let (program, _) = crate::parse("let s = \"hello\"; s.to_upper();");
        let prog = crate::compiler::compile(&program).unwrap();
        let result = run_with(&prog, OverflowMode::Wrap, Dispatch::Match).unwrap();
        assert_eq!(
            value_repr(&result),
            value_repr(&Value::String("HELLO".to_string()))
        );
    }

    #[test]
    fn res3904_array_method_call_dispatches_to_builtin() {
        let (program, _) = crate::parse("let a = [1, 2]; a.len();");
        let prog = crate::compiler::compile(&program).unwrap();
        let result = run_with(&prog, OverflowMode::Wrap, Dispatch::Match).unwrap();
        assert_eq!(value_repr(&result), value_repr(&Value::Int(2)));
    }

    #[test]
    fn res3904_array_collect_is_identity_not_a_builtin_call() {
        let (program, _) = crate::parse("let a = [1, 2, 3]; a.collect();");
        let prog = crate::compiler::compile(&program).unwrap();
        let result = run_with(&prog, OverflowMode::Wrap, Dispatch::Match).unwrap();
        assert_eq!(
            value_repr(&result),
            value_repr(&Value::Array(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3)
            ]))
        );
    }

    #[test]
    fn res3904_unrecognized_method_on_container_is_type_mismatch() {
        assert_both_type_mismatch("let s = \"ab\"; s.not_a_real_method();");
    }

    #[test]
    fn res3904_container_method_calls_agree_across_dispatch_engines() {
        assert_both_eq("let s = \"hello\"; s.to_upper();");
        assert_both_eq("let s = \"ab\"; s.repeat(3);");
        assert_both_eq("let a = [1, 2]; a.len();");
        assert_both_eq("let a = [1, 2, 3]; a.collect();");
    }

    #[test]
    fn res3918_get_field_reads_enum_tuple_payload() {
        let variant = Value::EnumVariant {
            type_name: "E".into(),
            variant: "P".into(),
            payload: crate::EnumValuePayload::Tuple(vec![Value::Int(10), Value::Int(20)]),
        };
        // `match` lowers tuple binding to `GetField{"0"}` / `GetField{"1"}`.
        assert!(matches!(
            vm_get_field_value(variant.clone(), "0"),
            Ok(Value::Int(10))
        ));
        assert!(matches!(
            vm_get_field_value(variant.clone(), "1"),
            Ok(Value::Int(20))
        ));
        // Out-of-range index is an UnknownField error, not a panic.
        assert!(matches!(
            vm_get_field_value(variant, "2"),
            Err(VmError::UnknownField { .. })
        ));
    }

    #[test]
    fn res3918_get_field_reads_enum_named_payload() {
        let variant = Value::EnumVariant {
            type_name: "Shape".into(),
            variant: "Circle".into(),
            payload: crate::EnumValuePayload::Named(vec![("r".into(), Value::Int(7))]),
        };
        assert!(matches!(
            vm_get_field_value(variant.clone(), "r"),
            Ok(Value::Int(7))
        ));
        assert!(matches!(
            vm_get_field_value(variant, "missing"),
            Err(VmError::UnknownField { .. })
        ));
    }

    #[test]
    fn res3918_get_field_on_non_struct_non_enum_is_type_mismatch() {
        assert!(matches!(
            vm_get_field_value(Value::Int(5), "0"),
            Err(VmError::TypeMismatch("GetField (non-struct target)"))
        ));
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
            upvalue_source_slots: Box::default(),
            fails: Box::default(),
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
    fn res329_direct_make_closure_works() {
        // MakeClosure in the direct-threaded path should produce a Closure value.
        use crate::bytecode::Function;
        let mut main = Chunk::new();
        main.code.push(Op::MakeClosure {
            fn_idx: 0,
            upvalue_count: 0,
        });
        main.code.push(Op::Return);
        main.line_info.push(1);
        main.line_info.push(1);
        let mut body = Chunk::new();
        body.code.push(Op::Return);
        body.line_info.push(1);
        let p = Program {
            main,
            functions: vec![Function {
                name: "f".into(),
                arity: 0,
                local_count: 0,
                upvalue_source_slots: Box::default(),
                fails: Box::default(),
                chunk: body,
            }],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let v = run_with(&p, OverflowMode::Wrap, Dispatch::Direct).unwrap();
        match v {
            Value::Closure { fn_idx, .. } => assert_eq!(fn_idx, 0),
            other => panic!("expected Closure, got {:?}", other),
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
            Op::MakeEnumTuple {
                type_const: 0,
                variant_const: 0,
                arity: 0,
            },
            Op::MakeEnumNamed {
                type_const: 0,
                variant_const: 0,
                field_count: 0,
            },
            Op::GetField { name_const: 0 },
            Op::SetField { name_const: 0 },
            Op::Band,
            Op::Bor,
            Op::Bxor,
            Op::Shl,
            Op::Shr,
            Op::AssertFail,
            Op::MakeTuple { len: 0 },
            Op::CallClosure {
                arity: 0,
                source_slot: u16::MAX,
            },
            Op::TryUnwrap,
            Op::IterPrepare,
            Op::LoadGlobal(0),
            Op::StoreGlobal(0),
            Op::LoadIndexUnchecked,
            Op::StoreUpvalue {
                upvalue_idx: 0,
                local_slot: 0,
            },
            Op::AssertBool,
            Op::Pop,
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
                !std::ptr::eq(h as *const (), h_unreachable as *const ()),
                "op {:?} mapped to unreachable slot {}",
                op,
                idx
            );
        }
    }

    fn assert_string(v: Result<Value, VmError>, expected: &str) {
        match v {
            Ok(Value::String(s)) => {
                assert_eq!(
                    s, expected,
                    "string mismatch: expected {expected:?}, got {s:?}"
                )
            }
            Ok(other) => panic!("expected String({expected:?}), got {other:?}"),
            Err(e) => panic!("expected String({expected:?}), got VmError: {e}"),
        }
    }

    // ── String concat (Op::Add extended) ─────────────────────────────────────

    #[test]
    fn add_string_string_concatenates() {
        let src = r#"let s = "hello" + " world"; s"#;
        assert_string(compile_run(src), "hello world");
    }

    #[test]
    fn add_int_string_coerces_via_chunk() {
        use crate::bytecode::{Chunk, Program};
        let mut chunk = Chunk::new();
        let i = chunk.add_constant(Value::Int(1)).unwrap();
        let s = chunk.add_constant(Value::String("x".to_string())).unwrap();
        chunk.emit(Op::Const(i), 1);
        chunk.emit(Op::Const(s), 1);
        chunk.emit(Op::Add, 1);
        chunk.emit(Op::Return, 1);
        let prog = Program {
            main: chunk,
            functions: vec![],
        };
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "1x"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    // ── Interpolated string (compiler + VM) ──────────────────────────────────

    #[test]
    fn interp_string_literal_only_parts() {
        // A string with no interpolations is a plain StringLiteral, not
        // InterpolatedString, but compile_run still handles it.
        assert_string(compile_run(r#""hello world""#), "hello world");
    }

    #[test]
    fn interp_string_single_var() {
        let src = r#"let name = "Alice"; "Hello, {name}!""#;
        assert_string(compile_run(src), "Hello, Alice!");
    }

    #[test]
    fn interp_string_arithmetic_expr() {
        let src = r#"let x = 6; let y = 7; "The answer is {x * y}.""#;
        assert_string(compile_run(src), "The answer is 42.");
    }

    #[test]
    fn interp_string_multiple_placeholders() {
        let src = r#"let a = 1; let b = 2; "{a} + {b} = {a + b}""#;
        assert_string(compile_run(src), "1 + 2 = 3");
    }

    #[test]
    fn interp_string_integer_conversion() {
        let src = r#"let n = 42; "n = {n}""#;
        assert_string(compile_run(src), "n = 42");
    }

    #[test]
    fn interp_string_bool_conversion() {
        let src = r#"let flag = true; "flag = {flag}""#;
        assert_string(compile_run(src), "flag = true");
    }

    // ── Tuple support (Op::MakeTuple + extended LoadIndex) ────────────────────

    #[test]
    fn tuple_literal_produces_tuple_value() {
        let src = "(1, 2, 3)";
        let r = compile_run(src);
        assert!(
            matches!(r, Ok(Value::Tuple(_))),
            "expected Tuple, got {r:?}"
        );
    }

    #[test]
    fn tuple_index_accesses_element() {
        let src = "let t = (10, 20, 30); t.1";
        let r = compile_run(src);
        assert!(matches!(r, Ok(Value::Int(20))), "expected 20, got {r:?}");
    }

    #[test]
    fn tuple_index_zero() {
        let src = "let t = (42, 99); t.0";
        let r = compile_run(src);
        assert!(matches!(r, Ok(Value::Int(42))), "expected 42, got {r:?}");
    }

    #[test]
    fn tuple_destructure_binds_all_names() {
        let src = "let (a, b, c) = (1, 2, 3); a + b + c";
        let r = compile_run(src);
        assert!(matches!(r, Ok(Value::Int(6))), "expected 6, got {r:?}");
    }

    #[test]
    fn tuple_destructure_two_elements() {
        let src = "let (x, y) = (10, 20); x * y";
        let r = compile_run(src);
        assert!(matches!(r, Ok(Value::Int(200))), "expected 200, got {r:?}");
    }

    #[test]
    fn tuple_empty_is_unit() {
        let src = "()";
        let r = compile_run(src);
        assert!(
            matches!(r, Ok(Value::Tuple(ref v)) if v.is_empty()),
            "expected empty Tuple, got {r:?}"
        );
    }

    // ── RES-163: match expression lowering ──────────────────────────────────

    #[test]
    fn match_wildcard_arm_always_matches() {
        let r = compile_run("fn f(int x) -> int { return match x { _ => 99 }; } f(42)");
        assert!(matches!(r, Ok(Value::Int(99))), "got {r:?}");
    }

    #[test]
    fn match_literal_arm_exact_hit() {
        let r = compile_run("fn f(int x) -> int { return match x { 5 => 100, _ => 0 }; } f(5)");
        assert!(matches!(r, Ok(Value::Int(100))), "got {r:?}");
    }

    #[test]
    fn match_literal_arm_miss_falls_to_wildcard() {
        let r = compile_run("fn f(int x) -> int { return match x { 5 => 100, _ => 0 }; } f(7)");
        assert!(matches!(r, Ok(Value::Int(0))), "got {r:?}");
    }

    #[test]
    fn match_identifier_binding_is_visible_in_body() {
        let r = compile_run("fn f(int x) -> int { return match x { n => n * 2 }; } f(6)");
        assert!(matches!(r, Ok(Value::Int(12))), "got {r:?}");
    }

    #[test]
    fn match_multiple_literal_arms_select_correctly() {
        let src = r#"
            fn grade(int n) -> int {
                return match n {
                    1 => 10,
                    2 => 20,
                    3 => 30,
                    _ => 0
                };
            }
            grade(2)
        "#;
        assert!(matches!(compile_run(src), Ok(Value::Int(20))));
    }

    #[test]
    fn match_no_arm_matched_yields_void() {
        // A match with only literal arms that all miss → fallthrough = Void.
        let r = compile_run("match 99 { 1 => 1 }");
        assert!(matches!(r, Ok(Value::Void)), "got {r:?}");
    }

    #[test]
    fn match_guard_skips_arm_when_false() {
        let src = r#"
            fn f(int x) -> int {
                return match x {
                    n if n > 10 => 1,
                    _ => 0
                };
            }
            f(5)
        "#;
        assert!(matches!(compile_run(src), Ok(Value::Int(0))));
    }

    #[test]
    fn match_guard_accepts_arm_when_true() {
        let src = r#"
            fn f(int x) -> int {
                return match x {
                    n if n > 10 => 1,
                    _ => 0
                };
            }
            f(15)
        "#;
        assert!(matches!(compile_run(src), Ok(Value::Int(1))));
    }

    // ── RES-155: struct destructuring in let ─────────────────────────────────

    #[test]
    fn let_destructure_struct_binds_field() {
        let src = r#"
            struct Point { int x, int y }
            let p = new Point { x: 10, y: 20 };
            let Point { x, y } = p;
            x + y
        "#;
        assert!(
            matches!(compile_run(src), Ok(Value::Int(30))),
            "got {:?}",
            compile_run(src)
        );
    }

    #[test]
    fn let_destructure_struct_renamed_field() {
        let src = r#"
            struct Vec2 { int x, int y }
            let v = new Vec2 { x: 3, y: 4 };
            let Vec2 { x: a, y: b } = v;
            a * a + b * b
        "#;
        assert!(
            matches!(compile_run(src), Ok(Value::Int(25))),
            "got {:?}",
            compile_run(src)
        );
    }

    #[test]
    fn let_destructure_struct_in_fn() {
        let src = r#"
            struct Pair { int first, int second }
            fn sum_pair(Pair p) -> int {
                let Pair { first, second } = p;
                return first + second;
            }
            sum_pair(new Pair { first: 7, second: 8 })
        "#;
        assert!(
            matches!(compile_run(src), Ok(Value::Int(15))),
            "got {:?}",
            compile_run(src)
        );
    }

    // ── RES-148/149: map and set literals ────────────────────────────────────

    #[test]
    fn map_literal_empty_is_map() {
        let r = compile_run("{}");
        assert!(matches!(r, Ok(Value::Map(_))), "expected Map, got {r:?}");
    }

    #[test]
    fn map_literal_with_entries_has_correct_len() {
        let r = compile_run(r#"let m = {"a" -> 1, "b" -> 2}; map_len(m)"#);
        assert!(matches!(r, Ok(Value::Int(2))), "got {r:?}");
    }

    #[test]
    fn map_literal_lookup_by_key() {
        let r = compile_run(r#"let m = {"hello" -> 42}; map_contains_key(m, "hello")"#);
        assert!(matches!(r, Ok(Value::Bool(true))), "got {r:?}");
    }

    #[test]
    fn set_literal_empty_is_set() {
        let r = compile_run("#{}");
        assert!(matches!(r, Ok(Value::Set(_))), "expected Set, got {r:?}");
    }

    #[test]
    fn set_literal_with_items_has_correct_len() {
        let r = compile_run("let s = #{1, 2, 3}; set_len(s)");
        assert!(matches!(r, Ok(Value::Int(3))), "got {r:?}");
    }

    #[test]
    fn set_literal_membership_check() {
        let r = compile_run("let s = #{10, 20, 30}; set_has(s, 20)");
        assert!(matches!(r, Ok(Value::Bool(true))), "got {r:?}");
    }

    // ---- RES-375/RES-923/RES-932: match patterns for Some/None/Ok/Err/Tuple ----

    #[test]
    fn match_some_pattern_extracts_value() {
        // Match arm bodies are expressions; Some(42) calls the Some builtin.
        let v =
            compile_run("let o = Some(42); return match o { Some(x) => x, None => -1 };").unwrap();
        assert_int(v, 42);
    }

    #[test]
    fn match_none_pattern_fires_on_absent_option() {
        // None() is the zero-arg builtin that creates an absent Option.
        let v =
            compile_run("let o = None(); return match o { Some(x) => x, None => -1 };").unwrap();
        assert_int(v, -1);
    }

    #[test]
    fn match_ok_pattern_extracts_payload() {
        let v = compile_run("let r = Ok(7); return match r { Ok(v) => v, Err(e) => -1 };").unwrap();
        assert_int(v, 7);
    }

    #[test]
    fn match_err_pattern_extracts_payload() {
        let v =
            compile_run("let r = Err(99); return match r { Ok(v) => v, Err(e) => e };").unwrap();
        assert_int(v, 99);
    }

    #[test]
    fn match_tuple_pattern_binds_elements() {
        let v = compile_run("let t = (10, 20); return match t { (a, b) => a + b };").unwrap();
        assert_int(v, 30);
    }

    #[test]
    fn match_tuple_pattern_wrong_length_falls_through() {
        // A 3-tuple doesn't match (a, b) — falls through to Wildcard.
        let v = compile_run("let t = (1, 2, 3); return match t { (a, b) => 0, _ => 1 };").unwrap();
        assert_int(v, 1);
    }

    #[test]
    fn div_min_by_neg1_wrap_mode() {
        let src = "return int_min() / -1;";
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        let v = run(&prog).unwrap();
        assert_int(v, i64::MIN);
    }

    #[test]
    fn mod_min_by_neg1_wrap_mode() {
        let src = "return int_min() % -1;";
        let (program, _) = crate::parse(src);
        let prog = crate::compiler::compile(&program).unwrap();
        let v = run(&prog).unwrap();
        assert_int(v, 0);
    }

    #[test]
    fn div_normal_case_unaffected() {
        let v = compile_run("return 10 / 3;").unwrap();
        assert_int(v, 3);
    }

    #[test]
    fn mod_normal_case_unaffected() {
        let v = compile_run("return 10 % 3;").unwrap();
        assert_int(v, 1);
    }

    #[test]
    fn overflow_mode_div_zero_error() {
        let mode = OverflowMode::Wrap;
        assert!(mode.div(42, 0).is_err());
    }

    #[test]
    fn overflow_mode_rem_zero_error() {
        let mode = OverflowMode::Wrap;
        assert!(mode.rem(42, 0).is_err());
    }

    #[test]
    fn overflow_mode_div_min_neg1_saturate() {
        let mode = OverflowMode::Saturate;
        assert_eq!(mode.div(i64::MIN, -1).unwrap(), i64::MAX);
    }

    #[test]
    fn overflow_mode_rem_min_neg1_saturate() {
        let mode = OverflowMode::Saturate;
        assert_eq!(mode.rem(i64::MIN, -1).unwrap(), 0);
    }

    #[test]
    fn overflow_mode_div_min_neg1_trap() {
        let mode = OverflowMode::Trap;
        assert!(matches!(
            mode.div(i64::MIN, -1),
            Err(VmError::IntegerOverflow(_))
        ));
    }

    #[test]
    fn overflow_mode_rem_min_neg1_trap() {
        let mode = OverflowMode::Trap;
        assert!(matches!(
            mode.rem(i64::MIN, -1),
            Err(VmError::IntegerOverflow(_))
        ));
    }

    // ---------- RES-2472: VM float arithmetic ----------

    #[test]
    fn vm_float_add() {
        let prog = const_program(
            &[Value::Float(1.5), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - 4.0).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_sub() {
        let prog = const_program(
            &[Value::Float(5.0), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Sub, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - 2.5).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_mul() {
        let prog = const_program(
            &[Value::Float(3.0), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - 7.5).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_div() {
        let prog = const_program(
            &[Value::Float(7.5), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Div, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - 3.0).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_div_by_zero_ieee754() {
        let prog = const_program(
            &[Value::Float(1.0), Value::Float(0.0)],
            &[Op::Const(0), Op::Const(1), Op::Div, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!(f.is_infinite()),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_mod() {
        let prog = const_program(
            &[Value::Float(7.5), Value::Float(2.0)],
            &[Op::Const(0), Op::Const(1), Op::Mod, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - 1.5).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_neg() {
        let prog = const_program(&[Value::Float(3.5)], &[Op::Const(0), Op::Neg, Op::Return]);
        match run(&prog).unwrap() {
            Value::Float(f) => assert!((f - (-3.5)).abs() < 1e-10),
            other => panic!("expected Float, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_comparison_lt() {
        let prog = const_program(
            &[Value::Float(1.5), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Lt, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_float_eq() {
        let prog = const_program(
            &[Value::Float(2.5), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_lt() {
        let prog = const_program(
            &[Value::String("abc".into()), Value::String("def".into())],
            &[Op::Const(0), Op::Const(1), Op::Lt, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "\"abc\" < \"def\" should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_gt() {
        let prog = const_program(
            &[Value::String("def".into()), Value::String("abc".into())],
            &[Op::Const(0), Op::Const(1), Op::Gt, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "\"def\" > \"abc\" should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_le_equal() {
        let prog = const_program(
            &[Value::String("abc".into()), Value::String("abc".into())],
            &[Op::Const(0), Op::Const(1), Op::Le, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "\"abc\" <= \"abc\" should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_ge_equal() {
        let prog = const_program(
            &[Value::String("abc".into()), Value::String("abc".into())],
            &[Op::Const(0), Op::Const(1), Op::Ge, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "\"abc\" >= \"abc\" should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_lt_false() {
        let prog = const_program(
            &[Value::String("xyz".into()), Value::String("abc".into())],
            &[Op::Const(0), Op::Const(1), Op::Lt, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(!b, "\"xyz\" < \"abc\" should be false"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_concat_int() {
        let prog = const_program(
            &[Value::String("x".into()), Value::Int(5)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "x5"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_int_concat_string() {
        let prog = const_program(
            &[Value::Int(42), Value::String("!".into())],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "42!"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_concat_float() {
        let prog = const_program(
            &[Value::String("val=".into()), Value::Float(2.5)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "val=2.5"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_concat_bool() {
        let prog = const_program(
            &[Value::String("b:".into()), Value::Bool(true)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "b:true"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_mul() {
        let prog = const_program(
            &[Value::String("ab".into()), Value::Int(3)],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "ababab"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_mul_commutative() {
        let prog = const_program(
            &[Value::Int(2), Value::String("xy".into())],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, "xyxy"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_mul_zero() {
        let prog = const_program(
            &[Value::String("abc".into()), Value::Int(0)],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::String(s) => assert_eq!(s, ""),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_string_mul_negative_errors() {
        let prog = const_program(
            &[Value::String("x".into()), Value::Int(-1)],
            &[Op::Const(0), Op::Const(1), Op::Mul, Op::Return],
        );
        let err = run(&prog).unwrap_err();
        match err.kind() {
            VmError::BuiltinCallFailed(msg) => {
                assert!(msg.contains("must be >= 0"), "got: {}", msg);
            }
            other => panic!("expected BuiltinCallFailed, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_arrays_equal() {
        let prog = const_program(
            &[
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "[1,2] == [1,2] should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_neq_arrays_different() {
        let prog = const_program(
            &[
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
                Value::Array(vec![Value::Int(1), Value::Int(3)]),
            ],
            &[Op::Const(0), Op::Const(1), Op::Neq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "[1,2] != [1,3] should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_arrays_different_length() {
        let prog = const_program(
            &[
                Value::Array(vec![Value::Int(1)]),
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(!b, "[1] == [1,2] should be false"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_tuples_equal() {
        let prog = const_program(
            &[
                Value::Tuple(vec![Value::Int(1), Value::String("a".into())]),
                Value::Tuple(vec![Value::Int(1), Value::String("a".into())]),
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "(1,\"a\") == (1,\"a\") should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_nested_arrays() {
        let prog = const_program(
            &[
                Value::Array(vec![
                    Value::Array(vec![Value::Int(1), Value::Int(2)]),
                    Value::Int(3),
                ]),
                Value::Array(vec![
                    Value::Array(vec![Value::Int(1), Value::Int(2)]),
                    Value::Int(3),
                ]),
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "nested arrays should be structurally equal"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_structs_equal() {
        let prog = const_program(
            &[
                Value::Struct {
                    name: "Point".into(),
                    fields: vec![("x".into(), Value::Int(1)), ("y".into(), Value::Int(2))],
                },
                Value::Struct {
                    name: "Point".into(),
                    fields: vec![("x".into(), Value::Int(1)), ("y".into(), Value::Int(2))],
                },
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "identical structs should be equal"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_neq_structs_different_name() {
        let prog = const_program(
            &[
                Value::Struct {
                    name: "Point".into(),
                    fields: vec![("x".into(), Value::Int(1))],
                },
                Value::Struct {
                    name: "Vec2".into(),
                    fields: vec![("x".into(), Value::Int(1))],
                },
            ],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(!b, "structs with different names should not be equal"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_eq_mismatched_types_is_type_mismatch() {
        // RES-3891: cross-kind `==` is a runtime type mismatch, matching the
        // tree-walking interpreter (`eval_infix` → "Type mismatch"). This test
        // previously asserted the divergent VM behavior — a silent `false` —
        // which is the exact soundness gap RES-3891 closes.
        let prog = const_program(
            &[Value::Int(1), Value::String("1".into())],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        let err = run(&prog).unwrap_err();
        assert!(
            matches!(err.kind(), VmError::TypeMismatch(_)),
            "Int vs String `==` should be a type mismatch, got {:?}",
            err.kind()
        );
    }

    #[test]
    fn vm_eq_map_equal() {
        use std::collections::HashMap;
        let mut m1 = HashMap::new();
        m1.insert(MapKey::Str("a".into()), Value::Int(1));
        m1.insert(MapKey::Int(2), Value::Bool(true));
        let mut m2 = HashMap::new();
        m2.insert(MapKey::Int(2), Value::Bool(true));
        m2.insert(MapKey::Str("a".into()), Value::Int(1));
        let prog = const_program(
            &[Value::Map(m1), Value::Map(m2)],
            &[Op::Const(0), Op::Const(1), Op::Eq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "maps with same entries should be equal"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_neq_map_different_value() {
        use std::collections::HashMap;
        let mut m1 = HashMap::new();
        m1.insert(MapKey::Str("a".into()), Value::Int(1));
        let mut m2 = HashMap::new();
        m2.insert(MapKey::Str("a".into()), Value::Int(2));
        let prog = const_program(
            &[Value::Map(m1), Value::Map(m2)],
            &[Op::Const(0), Op::Const(1), Op::Neq, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "maps with different values should not be equal"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_load_index_unchecked_in_bounds() {
        let prog = const_program(
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
                Op::LoadIndexUnchecked,
                Op::Return,
            ],
        );
        assert_int(run(&prog).unwrap(), 20);
    }

    #[test]
    fn vm_load_index_unchecked_negative_wraps() {
        let prog = const_program(
            &[
                Value::Int(10),
                Value::Int(20),
                Value::Int(30),
                Value::Int(-1),
            ],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::MakeArray { len: 3 },
                Op::Const(3),
                Op::LoadIndexUnchecked,
                Op::Return,
            ],
        );
        assert_int(run(&prog).unwrap(), 30);
    }

    #[test]
    fn vm_load_index_unchecked_out_of_bounds_errors() {
        let prog = const_program(
            &[Value::Int(10), Value::Int(20), Value::Int(5)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::MakeArray { len: 2 },
                Op::Const(2),
                Op::LoadIndexUnchecked,
                Op::Return,
            ],
        );
        let err = run(&prog).unwrap_err();
        assert!(
            matches!(
                err.kind(),
                VmError::ArrayIndexOutOfBounds { index: 5, len: 2 }
            ),
            "expected ArrayIndexOutOfBounds, got {:?}",
            err
        );
    }

    #[test]
    fn vm_load_index_unchecked_negative_out_of_bounds_errors() {
        let prog = const_program(
            &[Value::Int(10), Value::Int(20), Value::Int(-3)],
            &[
                Op::Const(0),
                Op::Const(1),
                Op::MakeArray { len: 2 },
                Op::Const(2),
                Op::LoadIndexUnchecked,
                Op::Return,
            ],
        );
        let err = run(&prog).unwrap_err();
        assert!(
            matches!(
                err.kind(),
                VmError::ArrayIndexOutOfBounds { index: -3, len: 2 }
            ),
            "expected ArrayIndexOutOfBounds, got {:?}",
            err
        );
    }

    #[test]
    fn vm_not_int_zero_is_true() {
        let prog = const_program(&[Value::Int(0)], &[Op::Const(0), Op::Not, Op::Return]);
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "!0 should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_load_index_negative_wraps_array() {
        let result = compile_run("let a = [10, 20, 30]; a[-1]").unwrap();
        assert_int(result, 30);
    }

    #[test]
    fn vm_load_index_negative_wraps_array_minus2() {
        let result = compile_run("let a = [10, 20, 30]; a[-2]").unwrap();
        assert_int(result, 20);
    }

    #[test]
    fn vm_load_index_negative_out_of_range() {
        let result = compile_run("let a = [10, 20, 30]; a[-4]");
        assert!(result.is_err(), "a[-4] on 3-element array should error");
    }

    #[test]
    fn vm_load_index_negative_wraps_string() {
        // RES-3889: negative string subscript wraps to the last char and
        // yields a `Value::Char` (was `Value::String` before the fix).
        let result = compile_run(r#"let s = "hello"; s[-1]"#).unwrap();
        match result {
            Value::Char(c) => assert_eq!(c, 'o'),
            other => panic!("expected Char, got {:?}", other),
        }
    }

    #[test]
    fn vm_not_int_nonzero_is_false() {
        let prog = const_program(&[Value::Int(42)], &[Op::Const(0), Op::Not, Op::Return]);
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(!b, "!42 should be false"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_not_float_zero_is_true() {
        let prog = const_program(&[Value::Float(0.0)], &[Op::Const(0), Op::Not, Op::Return]);
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "!0.0 should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_not_empty_string_is_true() {
        let prog = const_program(
            &[Value::String("".into())],
            &[Op::Const(0), Op::Not, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(b, "!\"\" should be true"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_not_nonempty_string_is_false() {
        let prog = const_program(
            &[Value::String("hello".into())],
            &[Op::Const(0), Op::Not, Op::Return],
        );
        match run(&prog).unwrap() {
            Value::Bool(b) => assert!(!b, "!\"hello\" should be false"),
            other => panic!("expected Bool, got {:?}", other),
        }
    }

    #[test]
    fn vm_jump_if_false_float_truthy() {
        let prog = const_program(
            &[Value::Float(1.5), Value::Int(1), Value::Int(0)],
            &[
                Op::Const(0),
                Op::JumpIfFalse(2),
                Op::Const(1),
                Op::Return,
                Op::Const(2),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(1) => {}
            other => panic!("expected Int(1) for truthy float, got {:?}", other),
        }
    }

    #[test]
    fn vm_jump_if_false_float_zero_is_falsy() {
        let prog = const_program(
            &[Value::Float(0.0), Value::Int(1), Value::Int(0)],
            &[
                Op::Const(0),
                Op::JumpIfFalse(2),
                Op::Const(1),
                Op::Return,
                Op::Const(2),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(0) => {}
            other => panic!("expected Int(0) for falsy 0.0, got {:?}", other),
        }
    }

    #[test]
    fn vm_jump_if_false_string_truthy() {
        let prog = const_program(
            &[Value::String("hello".into()), Value::Int(1), Value::Int(0)],
            &[
                Op::Const(0),
                Op::JumpIfFalse(2),
                Op::Const(1),
                Op::Return,
                Op::Const(2),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(1) => {}
            other => panic!("expected Int(1) for truthy string, got {:?}", other),
        }
    }

    #[test]
    fn vm_jump_if_false_empty_string_is_falsy() {
        let prog = const_program(
            &[Value::String("".into()), Value::Int(1), Value::Int(0)],
            &[
                Op::Const(0),
                Op::JumpIfFalse(2),
                Op::Const(1),
                Op::Return,
                Op::Const(2),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(0) => {}
            other => panic!("expected Int(0) for falsy empty string, got {:?}", other),
        }
    }

    #[test]
    fn vm_jump_if_true_float_truthy() {
        let prog = const_program(
            &[Value::Float(2.5), Value::Int(1), Value::Int(0)],
            &[
                Op::Const(0),
                Op::JumpIfTrue(2),
                Op::Const(2),
                Op::Return,
                Op::Const(1),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(1) => {}
            other => panic!(
                "expected Int(1) for truthy float via JumpIfTrue, got {:?}",
                other
            ),
        }
    }

    #[test]
    fn vm_jump_if_false_array_is_truthy() {
        let prog = const_program(
            &[
                Value::Array(vec![Value::Int(1)]),
                Value::Int(1),
                Value::Int(0),
            ],
            &[
                Op::Const(0),
                Op::JumpIfFalse(2),
                Op::Const(1),
                Op::Return,
                Op::Const(2),
                Op::Return,
            ],
        );
        match run(&prog).unwrap() {
            Value::Int(1) => {}
            other => panic!("expected Int(1) for truthy array, got {:?}", other),
        }
    }

    #[test]
    fn vm_load_index_negative_string_out_of_range() {
        let result = compile_run(r#"let s = "hi"; s[-3]"#);
        assert!(result.is_err(), "s[-3] on 2-char string should error");
    }

    #[test]
    fn vm_load_index_map_string_key() {
        let result = compile_run(r#"let m = {"a" -> 42, "b" -> 99}; m["a"]"#).unwrap();
        assert_int(result, 42);
    }

    #[test]
    fn vm_load_index_map_int_key() {
        let result = compile_run(r#"let m = {1 -> "one", 2 -> "two"}; m[1]"#).unwrap();
        match result {
            Value::String(s) => assert_eq!(s, "one"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn vm_load_index_map_missing_key_returns_void() {
        let result = compile_run(r#"let m = {"a" -> 1}; m["missing"]"#).unwrap();
        assert!(
            matches!(result, Value::Void),
            "missing key should return Void, got {:?}",
            result
        );
    }

    #[test]
    fn vm_store_index_map_string_key() {
        let result = compile_run(r#"let mut m = {"x" -> 0}; m["x"] = 77; m["x"]"#).unwrap();
        assert_int(result, 77);
    }

    #[test]
    fn vm_store_index_map_insert_new_key() {
        let result = compile_run(r#"let mut m = {"a" -> 1}; m["b"] = 2; m["b"]"#).unwrap();
        assert_int(result, 2);
    }

    // ── RES-2528: for-in over maps in the VM ────────────────────────

    #[test]
    fn vm_for_in_map_iterates_keys() {
        let src = r#"
            let m = {"b" -> 20, "a" -> 10};
            let result = [];
            for k in m {
                result = push(result, k);
            }
            result
        "#;
        let result = compile_run(src).unwrap();
        match result {
            Value::Array(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], Value::String(s) if s == "a"));
                assert!(matches!(&items[1], Value::String(s) if s == "b"));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn vm_for_in_map_access_values() {
        let src = r#"
            let m = {"x" -> 10, "y" -> 20};
            let total = 0;
            for k in m {
                total = total + m[k];
            }
            total
        "#;
        let result = compile_run(src).unwrap();
        assert_int(result, 30);
    }

    #[test]
    fn vm_for_in_map_empty() {
        let src = r#"
            let m = {};
            let count = 0;
            for k in m {
                count = count + 1;
            }
            count
        "#;
        let result = compile_run(src).unwrap();
        assert_int(result, 0);
    }

    #[test]
    fn vm_for_in_map_int_keys() {
        let src = r#"
            let m = {3 -> "c", 1 -> "a", 2 -> "b"};
            let result = [];
            for k in m {
                result = push(result, k);
            }
            result
        "#;
        let result = compile_run(src).unwrap();
        match result {
            Value::Array(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(&items[0], Value::Int(1)));
                assert!(matches!(&items[1], Value::Int(2)));
                assert!(matches!(&items[2], Value::Int(3)));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn vm_len_map() {
        let src = r#"len({"a" -> 1, "b" -> 2})"#;
        let result = compile_run(src).unwrap();
        assert_int(result, 2);
    }

    // ── RES-2530: ReturnFromCall with empty stack returns Void ───────

    #[test]
    fn vm_void_fn_ending_with_let() {
        let src = "fn foo() { let x = 1; } foo(); 42";
        let result = compile_run(src).unwrap();
        assert_int(result, 42);
    }

    #[test]
    fn vm_void_fn_ending_with_println() {
        let src = r#"fn foo() { println("hello"); } foo(); 42"#;
        let result = compile_run(src).unwrap();
        assert_int(result, 42);
    }

    #[test]
    fn vm_void_closure_ending_with_let() {
        let src = "let f = fn() { let y = 10; }; f(); 42";
        let result = compile_run(src).unwrap();
        assert_int(result, 42);
    }

    #[test]
    fn vm_explicit_return_still_returns_value() {
        let src = "fn f() -> int { return 99; } f()";
        let result = compile_run(src).unwrap();
        assert_int(result, 99);
    }

    #[test]
    fn vm_implicit_expression_return_still_works() {
        let src = "fn f() -> int { 42 } f()";
        let result = compile_run(src).unwrap();
        assert_int(result, 42);
    }

    // ── RES-2534: handler-table ↔ match-dispatch parity ─────────────────

    #[test]
    fn res2534_map_index_read_both_dispatch() {
        let src = r#"let m = {"a": 1, "b": 2}; m["a"]"#;
        assert_both_eq(src);
    }

    #[test]
    fn res2534_map_index_write_both_dispatch() {
        let src = r#"let m = {"x": 10}; m["x"] = 42; m["x"]"#;
        assert_both_eq(src);
    }

    #[test]
    fn res2534_negative_array_index_both_dispatch() {
        let src = "let a = [10, 20, 30]; a[-1]";
        assert_both_eq(src);
    }

    #[test]
    fn res2534_negative_array_store_both_dispatch() {
        let src = "let a = [10, 20, 30]; a[-1] = 99; a[2]";
        assert_both_eq(src);
    }

    // ── RES-3889: string subscript yields Char (interpreter parity) ─────

    #[test]
    fn res3889_string_subscript_is_char() {
        // `s[i]` must produce a `Value::Char`, matching the tree-walker
        // interpreter (RES-2709) — NOT a single-char `Value::String`.
        let src = r#"let s = "hello"; s[1]"#;
        match compile_run(src).unwrap() {
            Value::Char(c) => assert_eq!(c, 'e'),
            other => panic!("expected Char('e'), got {other:?}"),
        }
    }

    #[test]
    fn res3889_string_subscript_char_equals_char_literal() {
        // The observable symptom: `s[i] == 'c'` was false under --vm
        // because the subscript returned a String while the literal was
        // a Char. It must now be true on both dispatch engines.
        let src = r#"let s = "hello"; s[1] == 'e'"#;
        let (m, d) = run_both(src);
        assert_eq!(value_repr(&m.unwrap()), "Bool(true)");
        assert_eq!(value_repr(&d.unwrap()), "Bool(true)");
    }

    #[test]
    fn res3889_string_subscript_negative_index_is_char() {
        let src = r#"let s = "hello"; s[-1] == 'o'"#;
        assert_both_eq(src);
        assert_eq!(value_repr(&run_both(src).0.unwrap()), "Bool(true)");
    }

    #[test]
    fn res3889_string_plus_char_subscript_concat() {
        // `"x" + s[i]` must stringify the Char rather than hitting the
        // former `unreachable!()` in `vm_push_stringified`.
        let src = r#"let s = "hello"; "first=" + s[0]"#;
        assert_string(compile_run(src), "first=h");
        assert_both_eq(src);
    }

    #[test]
    fn res3889_char_subscript_plus_string_concat() {
        let src = r#"let s = "hello"; s[0] + "!""#;
        assert_string(compile_run(src), "h!");
        assert_both_eq(src);
    }

    #[test]
    fn res2534_string_repeat_limit_both_dispatch() {
        let src = r#""x" * 10000001"#;
        let (m, d) = run_both(src);
        assert!(m.is_err(), "match dispatch should error on huge repeat");
        assert!(d.is_err(), "direct dispatch should error on huge repeat");
    }

    #[test]
    fn res2534_string_repeat_ok_both_dispatch() {
        let src = r#""ab" * 3"#;
        assert_both_eq(src);
    }

    // ── RES-2536: closure upvalue mutation ───────────────────────────────

    #[test]
    fn res2536_closure_mutation_persists_across_calls() {
        let src = "let x = 0; let inc = fn() { x = x + 1; }; inc(); inc(); x";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 2);
    }

    #[test]
    fn res2536_closure_mutation_three_calls() {
        let src = "let x = 10; let dec = fn() { x = x - 1; }; dec(); dec(); dec(); x";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 7);
    }

    #[test]
    fn res2536_closure_captures_multiple_vars() {
        let src = "let a = 1; let b = 2; let swap = fn() { let t = a; a = b; b = t; }; swap(); a + b * 10";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 12);
    }

    #[test]
    fn res2536_closure_read_without_mutation_unchanged() {
        let src = "let x = 42; let read = fn() -> int { return x; }; read()";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 42);
    }

    // ── RES-2538: nested function definitions ───────────────────────────

    #[test]
    fn res2538_nested_fn_basic() {
        let src = "fn outer() -> int { fn inner() -> int { return 42; } return inner(); } outer()";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 42);
    }

    #[test]
    fn res2538_nested_fn_with_args() {
        let src = "fn outer(int x) -> int { fn double(int n) -> int { return n * 2; } return double(x); } outer(5)";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 10);
    }

    #[test]
    fn res2538_multiple_nested_fns() {
        let src = "fn outer(int x) -> int { fn dbl(int n) -> int { return n * 2; } fn inc(int n) -> int { return n + 1; } return inc(dbl(x)); } outer(5)";
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 11);
    }

    #[test]
    fn res2538_nested_fn_recursive() {
        let src = "fn fib(int n) -> int { fn go(int a, int b, int c) -> int { if c <= 0 { return a; } return go(b, a + b, c - 1); } return go(0, 1, n); } fib(10)";
        assert_int(compile_run(src).unwrap(), 55);
    }

    // ── RES-2540: struct/enum match patterns ──────────────────────────────

    #[test]
    fn res2540_struct_match_field_binding() {
        let src = r#"struct Point { int x, int y, }
let p = new Point { x: 3, y: 5 };
match p { Point { x, y } => x + y, _ => -1, }"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 8);
    }

    #[test]
    fn res2540_struct_match_literal_field() {
        let src = r#"struct Point { int x, int y, }
let p = new Point { x: 0, y: 7 };
match p { Point { x: 0, y } => y, _ => -1, }"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 7);
    }

    #[test]
    fn res2540_struct_match_fallthrough() {
        let src = r#"struct Point { int x, int y, }
let p = new Point { x: 5, y: 5 };
match p { Point { x: 0, y } => y, _ => 99, }"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 99);
    }

    #[test]
    fn res2540_enum_match_unit_variant() {
        let src = r#"enum Dir { N, S, E, W, }
let d = Dir::W;
match d { Dir::N => 1, Dir::S => 2, Dir::E => 3, Dir::W => 4, }"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 4);
    }

    #[test]
    fn res2540_enum_match_first_variant() {
        let src = r#"enum Dir { N, S, E, W, }
let d = Dir::N;
match d { Dir::N => 10, Dir::S => 20, Dir::E => 30, Dir::W => 40, }"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 10);
    }

    // ── RES-2542: impl method calls ──────────────────────────────────────

    #[test]
    fn res2542_impl_method_no_args() {
        let src = r#"struct Counter { int value, }
impl Counter { fn get(self) -> int { return self.value; } }
let c = new Counter { value: 42 };
c.get()"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 42);
    }

    #[test]
    fn res2542_impl_method_with_arg() {
        let src = r#"struct Counter { int value, }
impl Counter { fn add(self, int n) -> int { return self.value + n; } }
let c = new Counter { value: 10 };
c.add(5)"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 15);
    }

    #[test]
    fn res2542_impl_multiple_methods() {
        let src = r#"struct Point { int x, int y, }
impl Point {
    fn sum(self) -> int { return self.x + self.y; }
    fn scale(self, int factor) -> int { return self.sum() * factor; }
}
let p = new Point { x: 3, y: 4 };
p.scale(2)"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 14);
    }

    #[test]
    fn res2542_impl_method_chaining() {
        let src = r#"struct Val { int n, }
impl Val { fn doubled(self) -> int { return self.n * 2; } }
let a = new Val { n: 5 };
let b = new Val { n: a.doubled() };
b.doubled()"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 20);
    }

    #[test]
    fn res2544_try_catch_dispatches_to_handler() {
        let src = r#"fn read_sensor(int addr)
    requires addr >= 0
    fails Timeout
{
    return addr;
}

let result = 0;
try {
    let v = read_sensor(42);
    result = v;
} catch Timeout {
    result = -1;
}
result;"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), -1);
    }

    #[test]
    fn res2544_try_catch_nested_propagation() {
        let src = r#"fn read_sensor(int addr)
    requires addr >= 0
    fails Timeout, HardwareFault
{
    return addr;
}

let result = 0;
try {
    try {
        let v = read_sensor(42);
        result = v;
    } catch HardwareFault {
        result = -2;
    }
} catch Timeout {
    result = -1;
}
result;"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), -1);
    }

    #[test]
    fn res2544_try_catch_no_failure_runs_body() {
        let src = r#"fn safe_fn(int x)
    requires x >= 0
{
    return x * 2;
}

let result = 0;
try {
    result = safe_fn(21);
} catch Timeout {
    result = -1;
}
result;"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 42);
    }

    #[test]
    fn res2544_try_exit_cleans_try_stack() {
        let src = r#"fn safe_fn(int x)
    requires x >= 0
{
    return x + 1;
}

let a = 0;
try {
    a = safe_fn(10);
} catch Timeout {
    a = -1;
}
let b = safe_fn(20);
a + b;"#;
        assert_both_eq(src);
        assert_int(compile_run(src).unwrap(), 32);
    }
}
