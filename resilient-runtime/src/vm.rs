//! RES-3987 (D-E1): `#![no_std]`, zero-heap, zero-panic bytecode VM
//! skeleton for the Int/Bool/Float subset.
//!
//! `docs/EMBEDDED_PIPELINE.md` audits the host bytecode VM's 54
//! `Op` variants and finds 31 of them "no_std-clean" — they only
//! ever touch `Int`/`Bool`/`Float` operands and flat index/offset
//! data, never a heap-bearing `Value` variant (`String`, `Array`,
//! `Struct`, `Map`, `Set`, `Closure`, ...). This module is the
//! first increment of porting that subset into `resilient-runtime`:
//! arithmetic, comparisons, jumps, local load/store, push-const,
//! and return.
//!
//! # Deliberately self-contained
//!
//! This is a **fresh** instruction set and value type, not an
//! import of `resilient/src/bytecode.rs` / `resilient/src/vm.rs`.
//! Those live in the `std`-only host crate and dispatch against the
//! host `Value` enum (heap-bearing `String`/`Array`/`Struct`/...),
//! which cannot compile under `#![no_std]`. A later PR unifies the
//! encodings once the host and embedded value layers converge
//! (tracked by the `vm-alloc` follow-up in the design doc). Until
//! then [`Instr`] and [`vm::Value`](Value) here are independent of
//! [`crate::Value`] (the crate-root value type, which already gains
//! a heap-bearing `String` variant under the `alloc` feature — a
//! shape this VM deliberately does not accept).
//!
//! # Stack model
//!
//! No heap, no `Vec`. The operand stack, the per-frame locals slab,
//! and (RES-4077, D-E1 fn-support) the call-frame stack are all
//! fixed-capacity arrays sized by `const` generics, mirroring the
//! `[TimerState; MAX_TIMERS]` fixed-array idiom already used by
//! [`crate::timer`] and the `Fixed<N, D>` const-generic idiom used
//! by [`crate::fixed`]. `Instr::Call`/[`Vm::run_with_functions`]
//! push a bounded [`FunctionDef`] call frame instead of `TailCall`/
//! `ReturnFromCall` — see [`Vm`]'s docs for the `CALLS` bound and
//! [`VmError::CallStackOverflow`] for how unbounded/too-deep
//! recursion surfaces as a typed error rather than a stack smash.
//!
//! ```
//! use resilient_runtime::vm::{Instr, Value, Vm};
//!
//! // 1 + 2 * 3
//! let program = [
//!     Instr::PushConst(Value::Int(1)),
//!     Instr::PushConst(Value::Int(2)),
//!     Instr::PushConst(Value::Int(3)),
//!     Instr::Mul,
//!     Instr::Add,
//!     Instr::Return,
//! ];
//! let mut vm = Vm::<16, 4>::new();
//! assert_eq!(vm.run(&program), Ok(Value::Int(7)));
//! ```
//!
//! # No-panic guarantee
//!
//! Every fallible op returns `Result<_, VmError>` — stack
//! overflow/underflow, an out-of-range local slot index, a
//! jump/fetch past the end of the program, integer division or
//! modulo by zero, and operand-type mismatches are all typed
//! errors. Non-test code in this module has no `unwrap()` /
//! `expect()` / `panic!()` / indexing operator that can panic
//! (`get`/`get_mut` with an explicit `Result`-mapped `None` arm are
//! used throughout instead of `[]`).

// RES-3987 (D-E1): the `.rzbc` wire format — a compact, zero-heap,
// zero-panic serialization of an [`Instr`] stream that a thin
// on-device loader reconstructs. See [`serde`] for the byte layout.
pub mod serde;

// RES-3987 (D-E1): the on-device loader — decode() + Vm::run() glue
// that turns an embedded `.rzbc` byte blob into an executed result.
// See [`loader`] for `load_and_run` and [`loader::LoaderError`].
pub mod loader;

/// A VM operand value, limited to the no_std-clean scalar subset
/// audited in `docs/EMBEDDED_PIPELINE.md` section 1: `Int(i64)`,
/// `Bool(bool)`, `Float(f64)`. No `String`, no collections, no
/// closures — those all require a heap-bearing host `Value`
/// variant this VM does not have.
///
/// `Copy` because every variant is stack-only data; this lets the
/// operand stack and locals slab be plain fixed-capacity arrays
/// without a placeholder/`Option` dance for uninitialised slots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Float(f64),
}

impl Value {
    /// `self + rhs`. Wrapping `i64` add / IEEE-754 `f64` add,
    /// matching the wrap-on-overflow contract documented on
    /// [`crate::Value::add`] and the host bytecode VM's `Op::Add`.
    pub fn add(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_add(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
            _ => Err(VmError::TypeMismatch("add")),
        }
    }

    /// `self - rhs`. Wrapping `i64` sub / `f64` sub.
    pub fn sub(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_sub(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
            _ => Err(VmError::TypeMismatch("sub")),
        }
    }

    /// `self * rhs`. Wrapping `i64` mul / `f64` mul.
    pub fn mul(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_mul(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
            _ => Err(VmError::TypeMismatch("mul")),
        }
    }

    /// `self / rhs`. `Int / Int` errors on `rhs == 0` (typed
    /// error, never a panic — bare `i64::MIN / -1` would also
    /// overflow-panic in a checked build, so the `Int` arm always
    /// routes through `wrapping_div` once it's known `rhs != 0`).
    /// `Float / Float` follows IEEE-754 and never errors (produces
    /// inf or NaN), matching [`crate::Value::div`].
    pub fn div(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(VmError::DivideByZero),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_div(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
            _ => Err(VmError::TypeMismatch("div")),
        }
    }

    /// `self % rhs`. `Int % Int` errors on `rhs == 0`. `Float %
    /// Float` uses `core::ops::Rem` (available without `libm` —
    /// float remainder is a core-provided operator, not a
    /// transcendental function).
    pub fn rem(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => Err(VmError::DivideByZero),
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a.wrapping_rem(b))),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a % b)),
            _ => Err(VmError::TypeMismatch("rem")),
        }
    }

    /// Unary negation. `Int` uses `wrapping_neg` (so `-i64::MIN`
    /// wraps to `i64::MIN` instead of panicking); `Float` negates
    /// per IEEE-754. `Bool` has no negation — that's `not`.
    pub fn neg(self) -> Result<Value, VmError> {
        match self {
            Value::Int(a) => Ok(Value::Int(a.wrapping_neg())),
            Value::Float(a) => Ok(Value::Float(-a)),
            Value::Bool(_) => Err(VmError::TypeMismatch("neg")),
        }
    }

    /// Boolean negation. `Bool` only.
    pub fn not(self) -> Result<Value, VmError> {
        match self {
            Value::Bool(a) => Ok(Value::Bool(!a)),
            _ => Err(VmError::TypeMismatch("not")),
        }
    }

    /// `self == rhs`, producing a `Value::Bool`. Same-type
    /// compares only; mixed types are a `TypeMismatch` (matches
    /// the host VM's strict comparison). Float equality uses bit
    /// comparison (`to_bits`) so `NaN == NaN`, consistent with
    /// [`crate::Value::eq`] and the host VM's constant-pool dedup.
    pub fn veq(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a == b)),
            (Value::Bool(a), Value::Bool(b)) => Ok(Value::Bool(a == b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a.to_bits() == b.to_bits())),
            _ => Err(VmError::TypeMismatch("eq")),
        }
    }

    /// `self != rhs`. Defined as `!veq(rhs)` so the two stay in
    /// sync by construction.
    pub fn vneq(self, rhs: Value) -> Result<Value, VmError> {
        match self.veq(rhs)? {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            _ => unreachable!("veq always returns Value::Bool"),
        }
    }

    /// `self < rhs`. Numeric types only (`Int`/`Float`); `Bool`
    /// has no ordering.
    pub fn lt(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
            _ => Err(VmError::TypeMismatch("lt")),
        }
    }

    /// `self <= rhs`. Numeric types only.
    pub fn le(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
            _ => Err(VmError::TypeMismatch("le")),
        }
    }

    /// `self > rhs`. Numeric types only.
    pub fn gt(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
            _ => Err(VmError::TypeMismatch("gt")),
        }
    }

    /// `self >= rhs`. Numeric types only.
    pub fn ge(self, rhs: Value) -> Result<Value, VmError> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
            (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
            _ => Err(VmError::TypeMismatch("ge")),
        }
    }

    /// Truthiness for a conditional jump. Only `Bool` is
    /// accepted — this VM's scalar-only value set has no
    /// non-empty-string/non-empty-collection truthy cases to
    /// worry about (those only exist for heap-bearing `Value`
    /// variants the design doc excludes from this subset), so
    /// `JumpIfFalse`/`JumpIfTrue` reject anything but an explicit
    /// `Bool` rather than guessing.
    fn as_bool(self) -> Result<bool, VmError> {
        match self {
            Value::Bool(b) => Ok(b),
            _ => Err(VmError::TypeMismatch("branch condition")),
        }
    }
}

/// One VM instruction. Fixed-width (the enum discriminant plus at
/// most one `u16`/`u32`/[`Value`] operand), `Copy`, no heap operand
/// ever — mirrors the "no variable-length instruction decode"
/// rationale in the design doc's artifact-format section, even
/// though this in-memory form (not yet a serialized blob) doesn't
/// need the explicit byte layout that section specifies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Instr {
    /// Push an immediate constant.
    PushConst(Value),
    /// Push `locals[idx]`.
    LoadLocal(u16),
    /// Pop TOS, store into `locals[idx]`.
    StoreLocal(u16),
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// Unary negate TOS.
    Neg,
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    /// Unary boolean negate TOS.
    Not,
    /// Unconditional jump to an absolute instruction index.
    Jump(u32),
    /// Pop TOS (must be `Bool`); jump to `target` if `false`.
    JumpIfFalse(u32),
    /// Pop TOS (must be `Bool`); jump to `target` if `true`.
    JumpIfTrue(u32),
    /// Pop TOS and end execution, returning it as the program's
    /// result.
    Return,
    /// RES-4077 (D-E1 fn-support): call function-table index `idx`.
    /// Pops `arity` args off the operand stack (rightmost popped
    /// first, matching the host VM's `Op::Call`), pushes a call
    /// frame, and transfers control to the callee's code at pc 0.
    /// Only meaningful with [`Vm::run_with_functions`] — a bare
    /// [`Vm::run`] has no function table (`CALLS` defaults to 1, so
    /// there is no room to push a callee frame) and always surfaces
    /// [`VmError::CallStackOverflow`] for this instruction.
    Call(u16),
    /// RES-4075 (D-E1 fn-support tail): pop and discard TOS — a
    /// discarded expression-statement result, e.g. a `f(x);` call
    /// whose value is unused. The host compiler emits `Op::Pop`
    /// after every non-final expression statement, so any embedded
    /// program with more than one top-level statement needs this.
    Pop,
    /// RES-4075 (D-E1 fn-support tail): tail call to function-table
    /// index `idx`. Pops `arity` args like [`Instr::Call`], but
    /// *reuses* the current frame instead of pushing a new one —
    /// the host peephole pass rewrites a self-recursive
    /// `Call(i); Return` pair into `TailCall(i)` (see
    /// `resilient/src/compiler.rs`), so tail-recursive loops run in
    /// O(1) call-frame space and never hit
    /// [`VmError::CallStackOverflow`], no matter the depth.
    TailCall(u16),
    /// RES-4083 (D-E1 tail): enter a `try { }` block, pushing a
    /// try-handler frame that records where to unwind to and which
    /// [`TryHandlerEntry`] (in the table passed to
    /// [`Vm::run_with_tries`]) names the catch arms in scope. Mirrors
    /// the host VM's `Op::EnterTry` — see
    /// `resilient/src/rzbc_emit.rs` for how the host's per-chunk
    /// `try_handlers` tables get flattened into one global table with
    /// this instruction's `idx` pointing into it.
    EnterTry(u16),
    /// RES-4083 (D-E1 tail): exit a `try { }` block on normal
    /// completion — pops the topmost try-handler frame. A no-op
    /// (never an error) if the try stack happens to already be empty,
    /// mirroring the host VM's tolerant `Op::ExitTry`.
    ExitTry,
}

/// RES-4083 (D-E1 tail): one `catch Variant { }` arm — `variant` is a
/// compile-time-assigned numeric id for the failure-variant name (see
/// `rzbc_emit`'s variant interning table; the embedded wire format
/// has no string constant pool to carry the name itself), and
/// `handler_pc` is the absolute instruction index of the arm's body
/// in the *same code stream* as the `EnterTry` that owns this entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchArm {
    pub variant: u16,
    pub handler_pc: u32,
}

/// RES-4083 (D-E1 tail): the maximum number of `catch` arms a single
/// `try { }` block can declare in the embedded no_std VM. A fixed
/// bound (rather than a `TRY_ARMS` const generic) keeps
/// [`TryHandlerEntry`] a plain `Copy` value and keeps the `Vm` type
/// signature from growing a fourth const generic; a `try` block
/// wanting more arms is a typed [`crate::vm::VmError`]-adjacent
/// `rzbc_emit::EmitError` at compile time, not a silent truncation.
pub const MAX_CATCH_ARMS: usize = 4;

/// RES-4083 (D-E1 tail): one `try { }` block's catch-arm table, as
/// referenced by [`Instr::EnterTry`]. Stored in the flat table passed
/// to [`Vm::run_with_tries`] — see that table's module docs
/// (`vm::serde`) for how the host's per-chunk tables are flattened
/// into it. `arms[..arm_count]` are the populated entries.
#[derive(Debug, Clone, Copy)]
pub struct TryHandlerEntry {
    pub arms: [Option<CatchArm>; MAX_CATCH_ARMS],
}

impl TryHandlerEntry {
    /// An entry with no catch arms — never actually emitted (a `try`
    /// with zero `catch` arms is pointless), but a convenient
    /// placeholder for fixed-size buffer initialisation.
    pub const EMPTY: TryHandlerEntry = TryHandlerEntry {
        arms: [None; MAX_CATCH_ARMS],
    };

    fn find(&self, variant: u16) -> Option<u32> {
        self.arms
            .iter()
            .find_map(|arm| arm.filter(|a| a.variant == variant).map(|a| a.handler_pc))
    }
}

/// One callable function for [`Vm::run_with_functions`]: a
/// contiguous slice of [`Instr`] (the callee's own code, indexed
/// from 0 — mirrors how the host compiler emits per-function local
/// slots starting at 0), its arity, and its declared local-slot
/// count. Borrowed, not owned — no heap allocation needed, since a
/// `&[Instr]` is just a pointer + length.
#[derive(Debug, Clone, Copy)]
pub struct FunctionDef<'a> {
    pub code: &'a [Instr],
    pub arity: u8,
    pub local_count: u16,
    /// RES-4083 (D-E1 tail): function-table index of this function's
    /// synthesized postcondition-check function (`ensures`/
    /// `recovers_to` — see `compiler::build_postcheck_function` on
    /// the host side), or `None` if this function has no postcheck.
    /// The postcheck function is itself an ordinary [`FunctionDef`]
    /// entry (arity = this function's arity + 1, the extra slot
    /// holding the return value) — [`Vm::execute`] invokes it as an
    /// isolated nested call on every [`Instr::Return`] from this
    /// function, mirroring the host VM's `run_postcheck`.
    pub postcheck: Option<u16>,
    /// RES-4083 (D-E1 tail): this function's declared `fails`
    /// checked-failure variant, as the numeric id `rzbc_emit`
    /// interned it under, or `None` if the function declares no
    /// `fails` clause. Mirrors the host VM's "inject the *first*
    /// declared variant" semantics (`vm.rs`'s `h_call`) — Resilient's
    /// checked-failure injection only ever raises `func.fails[0]`, so
    /// there's no need to carry the whole list.
    pub fails_variant: Option<u16>,
}

/// Errors the VM can surface. Every fallible dispatch step returns
/// one of these instead of panicking — see the module-level
/// "No-panic guarantee" section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmError {
    /// The operand stack is full (`STACK` capacity reached) and an
    /// instruction tried to push another value.
    StackOverflow,
    /// The operand stack is empty and an instruction tried to pop
    /// a value.
    StackUnderflow,
    /// A `LoadLocal`/`StoreLocal` index was `>= LOCALS`.
    LocalsOutOfBounds,
    /// The program counter — either the next instruction to fetch,
    /// or a `Jump`/`JumpIfFalse`/`JumpIfTrue` target — pointed
    /// outside the bounds of the instruction slice. Covers both
    /// "malformed jump target" and "fell off the end of the
    /// program without hitting `Return`".
    PcOutOfBounds,
    /// `Int / 0` or `Int % 0`.
    DivideByZero,
    /// An op was applied to operand(s) of the wrong `Value`
    /// variant(s). The payload names the op, matching
    /// [`crate::RuntimeError::TypeMismatch`]'s shape.
    TypeMismatch(&'static str),
    /// RES-4077 (D-E1 fn-support): `Instr::Call(idx)` with `idx`
    /// outside the function table passed to
    /// [`Vm::run_with_functions`] (or any `Call` at all under a
    /// bare [`Vm::run`], which has no function table).
    FunctionOutOfBounds(u16),
    /// RES-4077 (D-E1 fn-support): a `Call` would push more nested
    /// call frames than the VM's `CALLS` capacity allows. This is
    /// the typed, non-panicking substitute for unbounded recursion
    /// — a deep or infinite recursive call always surfaces this
    /// error instead of overflowing a real stack.
    CallStackOverflow,
    /// RES-4083 (D-E1 tail): a function's synthesized postcheck
    /// (`ensures`/`recovers_to`) evaluated to `Value::Bool(false)`,
    /// or to a non-`Bool` value (the postcheck function's body must
    /// always produce a `Bool` — a non-`Bool` result is a
    /// translation bug, not a legitimate contract outcome, but is
    /// still surfaced as a typed error rather than a panic). Matches
    /// the host VM's "Contract violation in fn ..." abort semantics:
    /// a violation always aborts the whole run.
    PostcheckViolation,
    /// RES-4083 (D-E1 tail): `Instr::EnterTry` would push more nested
    /// try-handler frames than the VM's `TRIES` capacity allows —
    /// the typed substitute for unbounded `try` nesting depth.
    TryStackOverflow,
    /// RES-4083 (D-E1 tail): `Instr::EnterTry(idx)`/a `Call` into a
    /// `fails`-declaring function with `idx` outside the
    /// `try_handlers` table passed to [`Vm::run_with_tries`].
    TryHandlerOutOfBounds(u16),
    /// RES-4083 (D-E1 tail): a `Call` into a function declaring
    /// `fails` raised its checked-failure variant (numeric id) and no
    /// enclosing `try { }` block's catch arms matched it — mirrors
    /// the host VM's `VmError::CheckedFailure`. Always aborts the
    /// whole run, matching an uncaught checked failure on the host.
    CheckedFailure(u16),
}

/// RES-4077 (D-E1 fn-support): who to resume, and where, once the
/// frame that used this slot returns. Recorded on `Call`, consumed
/// on the matching `Return`. `caller_func: None` means "resume in
/// `program`" (the entry/main chunk); `Some(idx)` means "resume in
/// `functions[idx].code`".
#[derive(Debug, Clone, Copy)]
struct ReturnInfo {
    caller_func: Option<u16>,
    ret_pc: usize,
}

/// RES-4083 (D-E1 tail): an active `try { }` block, pushed by
/// `Instr::EnterTry` and consumed (or skipped past) by a subsequent
/// `Call` into a `fails`-declaring function. `call_depth`/`stack_depth`
/// are the frame index / operand-stack pointer to unwind back to on a
/// catch dispatch — captured at `EnterTry` time, exactly mirroring
/// the host VM's `TryFrame` snapshot.
#[derive(Debug, Clone, Copy)]
struct TryFrame {
    handler_idx: u16,
    call_depth: usize,
    stack_depth: usize,
}

/// A bytecode VM instance with a fixed-capacity operand stack
/// (`STACK` slots), a fixed-capacity per-frame local-variable slab
/// (`LOCALS` slots per frame), and a fixed-capacity call-frame
/// stack (`CALLS` simultaneous frames, main counted as frame 0).
/// All three bounds are compile-time `const` generic parameters —
/// no heap, no growth, overflow is a typed [`VmError`] rather than
/// a panic.
///
/// `CALLS` defaults to `1` (just the main/entry frame, no room to
/// push a callee) so every existing `Vm::<STACK, LOCALS>` call site
/// keeps compiling unchanged and behaves exactly as before —
/// `Instr::Call` under the default surfaces
/// [`VmError::CallStackOverflow`] rather than executing, since
/// there is no second frame slot. Programs that call functions
/// pick a `CALLS > 1` and use [`Vm::run_with_functions`].
///
/// Every frame gets the same `LOCALS`-sized slab regardless of its
/// declared local count — simpler and still zero-heap, at the cost
/// of some wasted memory relative to a tightly-packed bump
/// allocator (acceptable for v1; see RES-4077's PR description for
/// the follow-up note).
pub struct Vm<
    const STACK: usize,
    const LOCALS: usize,
    const CALLS: usize = 1,
    const TRIES: usize = 0,
> {
    stack: [Value; STACK],
    sp: usize,
    locals: [[Value; LOCALS]; CALLS],
    /// Index of the currently active frame; `0` is always `program`
    /// (the entry/main chunk).
    frame: usize,
    /// `returns[i]` is where frame `i` resumes its *caller* once
    /// frame `i` returns. Only slots `1..=frame` are meaningful at
    /// any given time; `returns[0]` is never read (frame 0 returning
    /// ends execution, see `Instr::Return`).
    returns: [ReturnInfo; CALLS],
    /// RES-4083 (D-E1 tail): `frame_func[i]` is the function-table
    /// index whose code frame `i` is executing (`None` for the
    /// `program`/main frame). Lets a checked-failure catch dispatch
    /// look up which code stream to resume in after unwinding back to
    /// an arbitrary ancestor frame — `returns` only records a frame's
    /// *caller*, not the frame's own identity, so this is tracked
    /// separately (mirrors the host VM's `TryFrame::chunk_idx`, just
    /// keyed by frame depth instead of carried on the try frame
    /// itself, since every embedded frame's function never changes
    /// once pushed).
    frame_func: [Option<u16>; CALLS],
    /// RES-4083 (D-E1 tail): active `try { }` blocks, `TRIES`
    /// simultaneous nesting depth. `TRIES` defaults to `0` so every
    /// existing `Vm::<STACK, LOCALS, CALLS>` call site keeps compiling
    /// unchanged — `Instr::EnterTry` under the default always
    /// surfaces `VmError::TryStackOverflow` (no room to push a try
    /// frame), and a `fails`-declaring function called with an empty
    /// try stack always propagates as `VmError::CheckedFailure`
    /// (never dispatches a catch), matching a program with no `try`
    /// blocks.
    try_stack: [TryFrame; TRIES],
    try_sp: usize,
}

impl<const STACK: usize, const LOCALS: usize, const CALLS: usize, const TRIES: usize> Default
    for Vm<STACK, LOCALS, CALLS, TRIES>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const STACK: usize, const LOCALS: usize, const CALLS: usize, const TRIES: usize>
    Vm<STACK, LOCALS, CALLS, TRIES>
{
    /// A fresh VM: empty operand stack, locals zero-initialised to
    /// `Value::Int(0)`, no active call frames beyond the implicit
    /// main frame, no active `try` blocks.
    pub fn new() -> Self {
        Self {
            stack: [Value::Int(0); STACK],
            sp: 0,
            locals: [[Value::Int(0); LOCALS]; CALLS],
            frame: 0,
            returns: [ReturnInfo {
                caller_func: None,
                ret_pc: 0,
            }; CALLS],
            frame_func: [None; CALLS],
            try_stack: [TryFrame {
                handler_idx: 0,
                call_depth: 0,
                stack_depth: 0,
            }; TRIES],
            try_sp: 0,
        }
    }

    /// Overwrite a slot in the *current* frame's locals slab before
    /// a run (e.g. to seed top-level `let`s or, before calling
    /// `run`/`run_with_functions`, the entry frame's initial
    /// state). Returns `LocalsOutOfBounds` if `idx >= LOCALS`
    /// instead of panicking.
    pub fn set_local(&mut self, idx: u16, value: Value) -> Result<(), VmError> {
        match self.locals[self.frame].get_mut(idx as usize) {
            Some(slot) => {
                *slot = value;
                Ok(())
            }
            None => Err(VmError::LocalsOutOfBounds),
        }
    }

    fn push(&mut self, value: Value) -> Result<(), VmError> {
        match self.stack.get_mut(self.sp) {
            Some(slot) => {
                *slot = value;
                self.sp += 1;
                Ok(())
            }
            None => Err(VmError::StackOverflow),
        }
    }

    fn pop(&mut self) -> Result<Value, VmError> {
        if self.sp == 0 {
            return Err(VmError::StackUnderflow);
        }
        let idx = self.sp - 1;
        match self.stack.get(idx) {
            Some(v) => {
                self.sp = idx;
                Ok(*v)
            }
            None => Err(VmError::StackUnderflow),
        }
    }

    fn binary(
        &mut self,
        f: impl FnOnce(Value, Value) -> Result<Value, VmError>,
    ) -> Result<(), VmError> {
        let b = self.pop()?;
        let a = self.pop()?;
        let result = f(a, b)?;
        self.push(result)
    }

    fn unary(&mut self, f: impl FnOnce(Value) -> Result<Value, VmError>) -> Result<(), VmError> {
        let a = self.pop()?;
        let result = f(a)?;
        self.push(result)
    }

    /// Run `program` from instruction 0 until `Return` or the
    /// first error, with no function table — `Instr::Call` always
    /// fails. Every branch, arithmetic op, and stack/locals access
    /// is bounds-checked — no panic path exists here.
    pub fn run(&mut self, program: &[Instr]) -> Result<Value, VmError> {
        self.execute(&[], &[], program, 0)
    }

    /// Run `program` from instruction 0 until the entry frame's
    /// `Return` or the first error, with `functions` available as
    /// the table `Instr::Call(idx)` indexes into. Requires
    /// `CALLS >= 2` for any `Call` to succeed (frame 0 is always
    /// `program`; a callee needs at least frame 1).
    pub fn run_with_functions(
        &mut self,
        functions: &[FunctionDef<'_>],
        program: &[Instr],
    ) -> Result<Value, VmError> {
        self.execute(functions, &[], program, 0)
    }

    /// RES-4083 (D-E1 tail): the `try`/`fails` counterpart of
    /// [`run_with_functions`](Self::run_with_functions) — additionally
    /// takes `try_handlers`, the flat table `Instr::EnterTry(idx)`
    /// indexes into (see `rzbc_emit` for how the host's per-chunk
    /// tables get flattened into this one global table). Requires
    /// `TRIES >= 1` for any `EnterTry` to succeed.
    pub fn run_with_tries(
        &mut self,
        functions: &[FunctionDef<'_>],
        try_handlers: &[TryHandlerEntry],
        program: &[Instr],
    ) -> Result<Value, VmError> {
        self.execute(functions, try_handlers, program, 0)
    }

    /// Dispatch loop, parameterised over `entry_frame` — the frame
    /// index this invocation starts and ends at. `entry_frame` is
    /// always `0` for the top-level [`run`](Self::run)/
    /// [`run_with_functions`](Self::run_with_functions) entry
    /// points; [`Instr::Return`]'s postcheck handling recurses into
    /// `execute` with `entry_frame` one past the returning callee's
    /// frame — see the `Instr::Return` arm below — so a nested
    /// postcheck evaluation runs as its own fully isolated call
    /// (fresh locals slab, shares the same bounded operand stack and
    /// terminates by popping exactly what it pushed, like any other
    /// well-formed function body) without needing a second `Vm`
    /// instance or heap allocation.
    fn execute<'a>(
        &mut self,
        functions: &'a [FunctionDef<'a>],
        try_handlers: &[TryHandlerEntry],
        program: &'a [Instr],
        entry_frame: usize,
    ) -> Result<Value, VmError> {
        self.frame = entry_frame;
        let mut current_func: Option<u16> = None;
        let mut code: &[Instr] = program;
        let mut pc: usize = 0;
        loop {
            let instr = *code.get(pc).ok_or(VmError::PcOutOfBounds)?;
            pc += 1;
            match instr {
                Instr::PushConst(v) => self.push(v)?,
                Instr::LoadLocal(idx) => {
                    let v = *self.locals[self.frame]
                        .get(idx as usize)
                        .ok_or(VmError::LocalsOutOfBounds)?;
                    self.push(v)?;
                }
                Instr::StoreLocal(idx) => {
                    let v = self.pop()?;
                    let slot = self.locals[self.frame]
                        .get_mut(idx as usize)
                        .ok_or(VmError::LocalsOutOfBounds)?;
                    *slot = v;
                }
                Instr::Add => self.binary(Value::add)?,
                Instr::Sub => self.binary(Value::sub)?,
                Instr::Mul => self.binary(Value::mul)?,
                Instr::Div => self.binary(Value::div)?,
                Instr::Rem => self.binary(Value::rem)?,
                Instr::Neg => self.unary(Value::neg)?,
                Instr::Eq => self.binary(Value::veq)?,
                Instr::Neq => self.binary(Value::vneq)?,
                Instr::Lt => self.binary(Value::lt)?,
                Instr::Le => self.binary(Value::le)?,
                Instr::Gt => self.binary(Value::gt)?,
                Instr::Ge => self.binary(Value::ge)?,
                Instr::Not => self.unary(Value::not)?,
                Instr::Jump(target) => {
                    pc = Self::validate_target(target, code.len())?;
                }
                Instr::JumpIfFalse(target) => {
                    let cond = self.pop()?.as_bool()?;
                    if !cond {
                        pc = Self::validate_target(target, code.len())?;
                    }
                }
                Instr::JumpIfTrue(target) => {
                    let cond = self.pop()?.as_bool()?;
                    if cond {
                        pc = Self::validate_target(target, code.len())?;
                    }
                }
                Instr::Call(idx) => {
                    let f = functions
                        .get(idx as usize)
                        .copied()
                        .ok_or(VmError::FunctionOutOfBounds(idx))?;
                    let arity = f.arity as usize;
                    // RES-4083 (D-E1 tail): mirrors the host VM's
                    // `h_call` — a call into a `fails`-declaring
                    // function made anywhere inside an active `try`
                    // block deterministically injects the function's
                    // first declared checked-failure variant instead
                    // of running the body at all. Dispatch (or an
                    // uncaught propagation) happens without ever
                    // pushing a new frame.
                    if self.try_sp > 0
                        && let Some(variant) = f.fails_variant
                    {
                        if self.sp < arity {
                            return Err(VmError::StackUnderflow);
                        }
                        self.sp -= arity;
                        pc = self.dispatch_checked_failure(
                            variant,
                            try_handlers,
                            &mut current_func,
                            &mut code,
                            functions,
                            program,
                        )?;
                        continue;
                    }
                    if self.sp < arity {
                        return Err(VmError::StackUnderflow);
                    }
                    let next_frame = self.frame + 1;
                    if next_frame >= CALLS {
                        return Err(VmError::CallStackOverflow);
                    }
                    for slot in self.locals[next_frame].iter_mut() {
                        *slot = Value::Int(0);
                    }
                    for i in (0..arity).rev() {
                        let v = self.pop()?;
                        let slot = self.locals[next_frame]
                            .get_mut(i)
                            .ok_or(VmError::LocalsOutOfBounds)?;
                        *slot = v;
                    }
                    self.returns[next_frame] = ReturnInfo {
                        caller_func: current_func,
                        ret_pc: pc,
                    };
                    current_func = Some(idx);
                    self.frame_func[next_frame] = Some(idx);
                    code = f.code;
                    pc = 0;
                    self.frame = next_frame;
                }
                Instr::EnterTry(idx) => {
                    if idx as usize >= try_handlers.len() {
                        return Err(VmError::TryHandlerOutOfBounds(idx));
                    }
                    if self.try_sp >= TRIES {
                        return Err(VmError::TryStackOverflow);
                    }
                    self.try_stack[self.try_sp] = TryFrame {
                        handler_idx: idx,
                        call_depth: self.frame,
                        stack_depth: self.sp,
                    };
                    self.try_sp += 1;
                }
                Instr::ExitTry => {
                    if self.try_sp > 0 {
                        self.try_sp -= 1;
                    }
                }
                Instr::Pop => {
                    self.pop()?;
                }
                Instr::TailCall(idx) => {
                    let f = functions
                        .get(idx as usize)
                        .copied()
                        .ok_or(VmError::FunctionOutOfBounds(idx))?;
                    let arity = f.arity as usize;
                    if self.sp < arity {
                        return Err(VmError::StackUnderflow);
                    }
                    // Reuse the current frame: pop args into its
                    // first `arity` slots, zero the rest (same
                    // fresh-frame hygiene as `Call`), and jump to
                    // the callee at pc 0. No `returns` push — the
                    // eventual `Return` resumes this frame's
                    // original caller.
                    for i in (0..arity).rev() {
                        let v = self.pop()?;
                        let slot = self.locals[self.frame]
                            .get_mut(i)
                            .ok_or(VmError::LocalsOutOfBounds)?;
                        *slot = v;
                    }
                    for slot in self.locals[self.frame].iter_mut().skip(arity) {
                        *slot = Value::Int(0);
                    }
                    current_func = Some(idx);
                    self.frame_func[self.frame] = Some(idx);
                    code = f.code;
                    pc = 0;
                }
                Instr::Return => {
                    let v = self.pop()?;
                    // RES-4083 (D-E1 tail): `current_func` at this
                    // point still names the function whose body is
                    // returning (it's only reassigned below, to the
                    // *caller*) — if it declares a postcheck
                    // (`ensures`/`recovers_to`), run it now, while
                    // `self.frame`'s locals (the callee's own
                    // parameters) are still live, mirroring the host
                    // VM's `run_postcheck` invocation from
                    // `Op::ReturnFromCall`. A violation aborts the
                    // whole run via `?`, matching the host's
                    // "Contract violation" abort.
                    if let Some(callee_idx) = current_func {
                        let callee = functions
                            .get(callee_idx as usize)
                            .ok_or(VmError::FunctionOutOfBounds(callee_idx))?;
                        if let Some(postcheck_idx) = callee.postcheck {
                            let arity = callee.arity as usize;
                            let next_frame = self.frame + 1;
                            if next_frame >= CALLS {
                                return Err(VmError::CallStackOverflow);
                            }
                            for i in 0..arity {
                                let arg = self.locals[self.frame]
                                    .get(i)
                                    .copied()
                                    .ok_or(VmError::LocalsOutOfBounds)?;
                                let slot = self.locals[next_frame]
                                    .get_mut(i)
                                    .ok_or(VmError::LocalsOutOfBounds)?;
                                *slot = arg;
                            }
                            let ret_slot = self.locals[next_frame]
                                .get_mut(arity)
                                .ok_or(VmError::LocalsOutOfBounds)?;
                            *ret_slot = v;
                            for slot in self.locals[next_frame].iter_mut().skip(arity + 1) {
                                *slot = Value::Int(0);
                            }
                            let postcheck_code = functions
                                .get(postcheck_idx as usize)
                                .ok_or(VmError::FunctionOutOfBounds(postcheck_idx))?
                                .code;
                            let returning_frame = self.frame;
                            self.frame_func[next_frame] = Some(postcheck_idx);
                            let outcome =
                                self.execute(functions, try_handlers, postcheck_code, next_frame)?;
                            self.frame = returning_frame;
                            match outcome {
                                Value::Bool(true) => {}
                                _ => return Err(VmError::PostcheckViolation),
                            }
                        }
                    }
                    if self.frame == entry_frame {
                        return Ok(v);
                    }
                    let info = self.returns[self.frame];
                    self.frame -= 1;
                    current_func = info.caller_func;
                    code = match current_func {
                        Some(fi) => {
                            functions
                                .get(fi as usize)
                                .ok_or(VmError::FunctionOutOfBounds(fi))?
                                .code
                        }
                        None => program,
                    };
                    pc = info.ret_pc;
                    self.push(v)?;
                }
            }
        }
    }

    /// RES-4083 (D-E1 tail): mirrors the host VM's `h_call` checked-
    /// failure unwind loop — pop try-handler frames from the newest
    /// down, and for the first one whose `TryHandlerEntry` has a
    /// `catch` arm matching `variant`, unwind `self.frame`/`self.sp`
    /// back to that block's snapshot, restore `current_func`/`code`
    /// to whatever was running in that frame, and resume at the
    /// arm's handler pc. If the whole try stack is exhausted with no
    /// match, the checked failure propagates as an uncaught
    /// `VmError::CheckedFailure` — never a panic, matching the host's
    /// abort-the-whole-run semantics for an undischarged `fails`
    /// variant.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_checked_failure<'a>(
        &mut self,
        variant: u16,
        try_handlers: &[TryHandlerEntry],
        current_func: &mut Option<u16>,
        code: &mut &'a [Instr],
        functions: &'a [FunctionDef<'a>],
        program: &'a [Instr],
    ) -> Result<usize, VmError> {
        while self.try_sp > 0 {
            self.try_sp -= 1;
            let try_frame = self.try_stack[self.try_sp];
            let entry = try_handlers
                .get(try_frame.handler_idx as usize)
                .ok_or(VmError::TryHandlerOutOfBounds(try_frame.handler_idx))?;
            if let Some(handler_pc) = entry.find(variant) {
                self.frame = try_frame.call_depth;
                self.sp = try_frame.stack_depth;
                *current_func = self.frame_func[self.frame];
                *code = match *current_func {
                    Some(fi) => {
                        functions
                            .get(fi as usize)
                            .ok_or(VmError::FunctionOutOfBounds(fi))?
                            .code
                    }
                    None => program,
                };
                return Self::validate_target(handler_pc, code.len());
            }
        }
        Err(VmError::CheckedFailure(variant))
    }

    fn validate_target(target: u32, len: usize) -> Result<usize, VmError> {
        let idx = target as usize;
        if idx < len {
            Ok(idx)
        } else {
            Err(VmError::PcOutOfBounds)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- arithmetic ----------

    #[test]
    fn add_int_constants() {
        let program = [
            Instr::PushConst(Value::Int(2)),
            Instr::PushConst(Value::Int(3)),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(5)));
    }

    #[test]
    fn arithmetic_precedence_program() {
        // 1 + 2 * 3 == 7
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Int(2)),
            Instr::PushConst(Value::Int(3)),
            Instr::Mul,
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(7)));
    }

    #[test]
    fn int_add_wraps_on_overflow_no_panic() {
        let program = [
            Instr::PushConst(Value::Int(i64::MAX)),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(i64::MIN)));
    }

    #[test]
    fn float_arithmetic() {
        let program = [
            Instr::PushConst(Value::Float(2.5)),
            Instr::PushConst(Value::Float(1.5)),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Float(4.0)));
    }

    #[test]
    fn neg_int_and_float() {
        let program = [Instr::PushConst(Value::Int(5)), Instr::Neg, Instr::Return];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(-5)));

        let program = [
            Instr::PushConst(Value::Float(1.5)),
            Instr::Neg,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Float(-1.5)));
    }

    #[test]
    fn neg_int_min_wraps_no_panic() {
        let program = [
            Instr::PushConst(Value::Int(i64::MIN)),
            Instr::Neg,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(i64::MIN)));
    }

    #[test]
    fn rem_int_and_float() {
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::PushConst(Value::Int(3)),
            Instr::Rem,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(1)));
    }

    // ---------- comparisons ----------

    #[test]
    fn comparisons_produce_bool() {
        let program = [
            Instr::PushConst(Value::Int(3)),
            Instr::PushConst(Value::Int(5)),
            Instr::Lt,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Bool(true)));
    }

    #[test]
    fn eq_uses_bit_compare_so_nan_equals_itself() {
        let program = [
            Instr::PushConst(Value::Float(f64::NAN)),
            Instr::PushConst(Value::Float(f64::NAN)),
            Instr::Eq,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Bool(true)));
    }

    #[test]
    fn not_negates_bool() {
        let program = [
            Instr::PushConst(Value::Bool(false)),
            Instr::Not,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Bool(true)));
    }

    // ---------- locals ----------

    #[test]
    fn store_and_load_local() {
        let program = [
            Instr::PushConst(Value::Int(42)),
            Instr::StoreLocal(0),
            Instr::LoadLocal(0),
            Instr::LoadLocal(0),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(84)));
    }

    #[test]
    fn set_local_seeds_before_run() {
        let mut vm = Vm::<8, 2>::new();
        vm.set_local(0, Value::Int(10)).unwrap();
        vm.set_local(1, Value::Int(20)).unwrap();
        let program = [
            Instr::LoadLocal(0),
            Instr::LoadLocal(1),
            Instr::Add,
            Instr::Return,
        ];
        assert_eq!(vm.run(&program), Ok(Value::Int(30)));
    }

    // ---------- control flow (loop via jump) ----------

    #[test]
    fn loop_sums_one_to_five_via_jump() {
        // locals[0] = i = 0; locals[1] = sum = 0
        // loop: if i >= 5 goto end
        //   sum += i; i += 1; goto loop
        // end: return sum
        //
        // idx: 0  PushConst(0)      -> i = 0
        //      1  StoreLocal(0)
        //      2  PushConst(0)      -> sum = 0
        //      3  StoreLocal(1)
        // loop:
        //      4  LoadLocal(0)
        //      5  PushConst(5)
        //      6  Lt                -> i < 5
        //      7  JumpIfFalse(end=17)
        //      8  LoadLocal(1)
        //      9  LoadLocal(0)
        //     10  Add
        //     11  StoreLocal(1)     -> sum += i
        //     12  LoadLocal(0)
        //     13  PushConst(1)
        //     14  Add
        //     15  StoreLocal(0)     -> i += 1
        //     16  Jump(4)
        // end:
        //     17  LoadLocal(1)
        //     18  Return
        let program = [
            Instr::PushConst(Value::Int(0)),
            Instr::StoreLocal(0),
            Instr::PushConst(Value::Int(0)),
            Instr::StoreLocal(1),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(5)),
            Instr::Lt,
            Instr::JumpIfFalse(17),
            Instr::LoadLocal(1),
            Instr::LoadLocal(0),
            Instr::Add,
            Instr::StoreLocal(1),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::StoreLocal(0),
            Instr::Jump(4),
            Instr::LoadLocal(1),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 2>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(1 + 2 + 3 + 4)));
    }

    #[test]
    fn jump_if_true_takes_branch() {
        let program = [
            Instr::PushConst(Value::Bool(true)),
            Instr::JumpIfTrue(4),
            Instr::PushConst(Value::Int(1)),
            Instr::Return,
            Instr::PushConst(Value::Int(2)),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(2)));
    }

    // ---------- error paths: never panic, always Result::Err ----------

    #[test]
    fn stack_overflow_is_a_typed_error_not_a_panic() {
        // Capacity 4; 5 pushes with no intervening pop must overflow
        // before the (unreachable) Return is fetched.
        let mut program = [Instr::PushConst(Value::Int(1)); 6];
        program[5] = Instr::Return;
        let mut vm = Vm::<4, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::StackOverflow));
    }

    #[test]
    fn stack_underflow_is_a_typed_error_not_a_panic() {
        let program = [Instr::Add, Instr::Return];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::StackUnderflow));
    }

    #[test]
    fn return_on_empty_stack_is_underflow_not_panic() {
        let program = [Instr::Return];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::StackUnderflow));
    }

    #[test]
    fn int_div_by_zero_is_a_typed_error_not_a_panic() {
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::PushConst(Value::Int(0)),
            Instr::Div,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::DivideByZero));
    }

    #[test]
    fn int_rem_by_zero_is_a_typed_error_not_a_panic() {
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::PushConst(Value::Int(0)),
            Instr::Rem,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::DivideByZero));
    }

    #[test]
    fn int_div_min_by_neg_one_wraps_without_panic() {
        let program = [
            Instr::PushConst(Value::Int(i64::MIN)),
            Instr::PushConst(Value::Int(-1)),
            Instr::Div,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(i64::MIN)));
    }

    #[test]
    fn float_div_by_zero_yields_inf_not_error() {
        let program = [
            Instr::PushConst(Value::Float(1.0)),
            Instr::PushConst(Value::Float(0.0)),
            Instr::Div,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        match vm.run(&program) {
            Ok(Value::Float(v)) => assert!(v.is_infinite()),
            other => panic!("expected Ok(Value::Float(inf)), got {other:?}"),
        }
    }

    #[test]
    fn locals_out_of_bounds_is_a_typed_error_not_a_panic() {
        let program = [Instr::LoadLocal(3), Instr::Return];
        let mut vm = Vm::<8, 2>::new();
        assert_eq!(vm.run(&program), Err(VmError::LocalsOutOfBounds));
    }

    #[test]
    fn store_local_out_of_bounds_is_a_typed_error_not_a_panic() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::StoreLocal(9),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 2>::new();
        assert_eq!(vm.run(&program), Err(VmError::LocalsOutOfBounds));
    }

    #[test]
    fn set_local_out_of_bounds_is_a_typed_error_not_a_panic() {
        let mut vm = Vm::<8, 2>::new();
        assert_eq!(
            vm.set_local(5, Value::Int(1)),
            Err(VmError::LocalsOutOfBounds)
        );
    }

    #[test]
    fn bad_jump_target_is_a_typed_error_not_a_panic() {
        let program = [Instr::Jump(999)];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::PcOutOfBounds));
    }

    #[test]
    fn falling_off_end_without_return_is_a_typed_error_not_a_panic() {
        let program = [Instr::PushConst(Value::Int(1))];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::PcOutOfBounds));
    }

    #[test]
    fn type_mismatch_add_int_bool_is_a_typed_error_not_a_panic() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Bool(true)),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::TypeMismatch("add")));
    }

    #[test]
    fn type_mismatch_branch_on_non_bool_is_a_typed_error_not_a_panic() {
        let program = [Instr::PushConst(Value::Int(1)), Instr::JumpIfFalse(0)];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(
            vm.run(&program),
            Err(VmError::TypeMismatch("branch condition"))
        );
    }

    #[test]
    fn lt_on_bool_is_type_mismatch() {
        let program = [
            Instr::PushConst(Value::Bool(true)),
            Instr::PushConst(Value::Bool(false)),
            Instr::Lt,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::TypeMismatch("lt")));
    }

    #[test]
    fn neg_on_bool_is_type_mismatch() {
        let program = [Instr::PushConst(Value::Bool(true)), Instr::Neg];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::TypeMismatch("neg")));
    }

    #[test]
    fn not_on_int_is_type_mismatch() {
        let program = [Instr::PushConst(Value::Int(1)), Instr::Not];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::TypeMismatch("not")));
    }

    #[test]
    fn zero_capacity_locals_still_runs_arithmetic() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(2)));
    }

    #[test]
    fn vneq_is_negation_of_veq() {
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::PushConst(Value::Int(2)),
            Instr::Neq,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Bool(true)));
    }

    // ---------- RES-4075 (fn-support tail): TailCall + Pop ----------

    #[test]
    fn pop_discards_unused_call_result() {
        // main: f(); 9   — f() = 5, result discarded via Pop.
        let f = [Instr::PushConst(Value::Int(5)), Instr::Return];
        let functions = [FunctionDef {
            code: &f,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [
            Instr::Call(0),
            Instr::Pop,
            Instr::PushConst(Value::Int(9)),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(9))
        );
    }

    #[test]
    fn pop_on_empty_stack_is_underflow_not_a_panic() {
        let program = [Instr::Pop, Instr::Return];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::StackUnderflow));
    }

    #[test]
    fn tail_call_recursion_runs_in_constant_frame_space() {
        // countdown(n) = if n < 1 { 0 } else { countdown(n - 1) }
        // in tail-call form: depth 100 with only CALLS = 2.
        let countdown = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Lt,
            Instr::JumpIfFalse(6),
            Instr::PushConst(Value::Int(0)),
            Instr::Return,
            Instr::LoadLocal(0), // 6
            Instr::PushConst(Value::Int(1)),
            Instr::Sub,
            Instr::TailCall(0),
        ];
        let functions = [FunctionDef {
            code: &countdown,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [
            Instr::PushConst(Value::Int(100)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(0))
        );
    }

    #[test]
    fn tail_call_returns_to_original_caller() {
        // g(x) = x * 2; f(x) = TailCall g(x + 1); main: 10 + f(3).
        // f's TailCall must return g's result to *main*, and main's
        // locals must be untouched by the reused frame.
        let g = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(2)),
            Instr::Mul,
            Instr::Return,
        ];
        let f = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::TailCall(1),
        ];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: None,
                fails_variant: None,
            },
            FunctionDef {
                code: &g,
                arity: 1,
                local_count: 1,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::StoreLocal(0),
            Instr::PushConst(Value::Int(3)),
            Instr::Call(0),
            Instr::LoadLocal(0),
            Instr::Add,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 3>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(18))
        );
    }

    #[test]
    fn tail_call_bad_function_index_is_typed_error_not_a_panic() {
        let f = [Instr::TailCall(7)];
        let functions = [FunctionDef {
            code: &f,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [Instr::Call(0), Instr::Return];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::FunctionOutOfBounds(7))
        );
    }

    #[test]
    fn tail_call_with_missing_args_is_underflow_not_a_panic() {
        let f = [Instr::TailCall(0)];
        let functions = [FunctionDef {
            code: &f,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        // Call f with its one arg; f then TailCalls itself with an
        // empty operand stack.
        let program = [
            Instr::PushConst(Value::Int(1)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::StackUnderflow)
        );
    }

    // ---------- RES-4077 (D-E1 fn-support): calls ----------

    #[test]
    fn call_returns_a_constant() {
        // fn f() -> Int { 42 }
        // main: f()
        let square = [Instr::PushConst(Value::Int(42)), Instr::Return];
        let functions = [FunctionDef {
            code: &square,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [Instr::Call(0), Instr::Return];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(42))
        );
    }

    #[test]
    fn call_passes_arguments_into_callee_locals() {
        // fn square(x: Int) -> Int { x * x }
        // main: square(7)
        let square = [
            Instr::LoadLocal(0),
            Instr::LoadLocal(0),
            Instr::Mul,
            Instr::Return,
        ];
        let functions = [FunctionDef {
            code: &square,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [
            Instr::PushConst(Value::Int(7)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(49))
        );
    }

    #[test]
    fn call_with_multiple_arguments_preserves_order() {
        // fn sub(a: Int, b: Int) -> Int { a - b }
        // main: sub(10, 3) == 7  (argument order must not be swapped
        // by the reverse-pop-off-the-stack fill loop)
        let sub = [
            Instr::LoadLocal(0),
            Instr::LoadLocal(1),
            Instr::Sub,
            Instr::Return,
        ];
        let functions = [FunctionDef {
            code: &sub,
            arity: 2,
            local_count: 2,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [
            Instr::PushConst(Value::Int(10)),
            Instr::PushConst(Value::Int(3)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(7))
        );
    }

    #[test]
    fn nested_calls_resume_correct_caller() {
        // fn inc(x: Int) -> Int { x + 1 }
        // fn double_inc(x: Int) -> Int { inc(inc(x)) }
        // main: double_inc(5) == 7
        let inc = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::Return,
        ];
        let double_inc = [
            Instr::LoadLocal(0),
            Instr::Call(0), // inc(x)
            Instr::Call(0), // inc(inc(x))
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &inc,
                arity: 1,
                local_count: 1,
                postcheck: None,
                fails_variant: None,
            },
            FunctionDef {
                code: &double_inc,
                arity: 1,
                local_count: 1,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(5)),
            Instr::Call(1),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 3>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(7))
        );
    }

    #[test]
    fn recursive_call_within_depth_budget_succeeds() {
        // fn countdown(n: Int) -> Int { if n <= 0 { n } else { countdown(n - 1) } }
        // main: countdown(3) == 0
        let countdown = [
            Instr::LoadLocal(0),             // 0: push n
            Instr::PushConst(Value::Int(0)), // 1: push 0
            Instr::Gt,                       // 2: n > 0
            Instr::JumpIfFalse(9),           // 3: -> base case
            Instr::LoadLocal(0),             // 4: push n
            Instr::PushConst(Value::Int(1)), // 5: push 1
            Instr::Sub,                      // 6: n - 1
            Instr::Call(0),                  // 7: countdown(n - 1)
            Instr::Return,                   // 8: return recursive result
            Instr::LoadLocal(0),             // 9: base case: push n
            Instr::Return,                   // 10
        ];
        let functions = [FunctionDef {
            code: &countdown,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        let program = [
            Instr::PushConst(Value::Int(3)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 8>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(0))
        );
    }

    #[test]
    fn recursion_beyond_call_depth_is_typed_error_not_a_panic() {
        let countdown = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::JumpIfFalse(9),
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Sub,
            Instr::Call(0),
            Instr::Return,
            Instr::LoadLocal(0),
            Instr::Return,
        ];
        let functions = [FunctionDef {
            code: &countdown,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        // CALLS == 3 only allows 2 nested frames (main + 2 callees);
        // recursing 100 deep must surface a typed error, never
        // overflow a real stack.
        let program = [
            Instr::PushConst(Value::Int(100)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 3>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::CallStackOverflow)
        );
    }

    #[test]
    fn call_to_out_of_range_function_index_is_typed_error() {
        let program = [
            Instr::PushConst(Value::Int(0)),
            Instr::Call(5),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0, 2>::new();
        assert_eq!(
            vm.run_with_functions(&[], &program),
            Err(VmError::FunctionOutOfBounds(5))
        );
    }

    #[test]
    fn call_under_bare_run_with_no_function_table_is_function_out_of_bounds() {
        // `Vm::run` passes an empty function table, so any `Call`
        // index — including 0 — is out of range. A bare `run` also
        // has `CALLS` defaulting to 1, leaving no room to push a
        // callee frame even if the table were non-empty; the
        // function-table check runs first and produces the more
        // specific error.
        let program = [Instr::Call(0), Instr::Return];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Err(VmError::FunctionOutOfBounds(0)));
    }

    #[test]
    fn call_with_too_few_stack_values_for_arity_is_stack_underflow() {
        let callee = [Instr::LoadLocal(0), Instr::Return];
        let functions = [FunctionDef {
            code: &callee,
            arity: 1,
            local_count: 1,
            postcheck: None,
            fails_variant: None,
        }];
        // No PushConst before Call(0) — the operand stack is empty
        // but the callee wants 1 argument.
        let program = [Instr::Call(0), Instr::Return];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::StackUnderflow)
        );
    }

    // ---------- RES-4083 (D-E1 tail): postcheck (ensures/recovers_to) ----------

    #[test]
    fn postcheck_runs_on_return_and_passes_when_result_satisfies_it() {
        // fn f(x) -> Int { x + 1 } ensures result > 0
        // postcheck(x, result) -> Bool { result > 0 }
        // main: f(5) == 6
        let f = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Add,
            Instr::Return,
        ];
        let postcheck = [
            Instr::LoadLocal(1),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: Some(1),
                fails_variant: None,
            },
            FunctionDef {
                code: &postcheck,
                arity: 2,
                local_count: 2,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(5)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 4>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(6))
        );
    }

    #[test]
    fn postcheck_violation_aborts_the_run_as_a_typed_error() {
        // fn f(x) -> Int { x } ensures result > 0 — violated for x <= 0.
        let f = [Instr::LoadLocal(0), Instr::Return];
        let postcheck = [
            Instr::LoadLocal(1),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: Some(1),
                fails_variant: None,
            },
            FunctionDef {
                code: &postcheck,
                arity: 2,
                local_count: 2,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(-1)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 4>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::PostcheckViolation)
        );
    }

    #[test]
    fn postcheck_non_bool_result_is_postcheck_violation_not_a_panic() {
        // A malformed postcheck body (translation bug, not a real
        // program) that yields a non-Bool must still be a typed
        // error, never a panic.
        let f = [Instr::LoadLocal(0), Instr::Return];
        let postcheck = [Instr::LoadLocal(1), Instr::Return];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: Some(1),
                fails_variant: None,
            },
            FunctionDef {
                code: &postcheck,
                arity: 2,
                local_count: 2,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(5)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 4>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::PostcheckViolation)
        );
    }

    #[test]
    fn postcheck_call_stack_overflow_is_typed_error_not_a_panic() {
        // CALLS == 2 leaves no room for the postcheck's own nested
        // frame once `f`'s frame is occupied.
        let f = [Instr::LoadLocal(0), Instr::Return];
        let postcheck = [
            Instr::LoadLocal(1),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: Some(1),
                fails_variant: None,
            },
            FunctionDef {
                code: &postcheck,
                arity: 2,
                local_count: 2,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(5)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Err(VmError::CallStackOverflow)
        );
    }

    #[test]
    fn nested_postcheck_calling_another_function_is_supported() {
        // postcheck(x, result) -> Bool { is_positive(result) }
        // is_positive(v) -> Bool { v > 0 }
        // fn f(x) -> Int { x } ensures is_positive(result)
        let f = [Instr::LoadLocal(0), Instr::Return];
        let is_positive = [
            Instr::LoadLocal(0),
            Instr::PushConst(Value::Int(0)),
            Instr::Gt,
            Instr::Return,
        ];
        let postcheck = [
            Instr::LoadLocal(1),
            Instr::Call(2), // is_positive(result)
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &f,
                arity: 1,
                local_count: 1,
                postcheck: Some(1),
                fails_variant: None,
            },
            FunctionDef {
                code: &postcheck,
                arity: 2,
                local_count: 2,
                postcheck: None,
                fails_variant: None,
            },
            FunctionDef {
                code: &is_positive,
                arity: 1,
                local_count: 1,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let program = [
            Instr::PushConst(Value::Int(5)),
            Instr::Call(0),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 5>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(5))
        );
    }

    // ---------- RES-4083 (D-E1 tail): `fails`/checked-failure dispatch ----------

    #[test]
    fn call_outside_try_declaring_fails_runs_normally() {
        // fn risky() fails Boom { 42 } — called with no enclosing try:
        // the checked-failure injection only fires inside a `try`, so
        // this just runs the body like any other call.
        let risky = [Instr::PushConst(Value::Int(42)), Instr::Return];
        let functions = [FunctionDef {
            code: &risky,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: Some(0),
        }];
        let program = [Instr::Call(0), Instr::Return];
        let mut vm = Vm::<8, 4, 2>::new();
        assert_eq!(
            vm.run_with_functions(&functions, &program),
            Ok(Value::Int(42))
        );
    }

    #[test]
    fn call_inside_try_dispatches_to_matching_catch_arm() {
        // try { risky(); } catch Boom { -1 }
        let risky = [Instr::PushConst(Value::Int(42)), Instr::Return];
        let functions = [FunctionDef {
            code: &risky,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: Some(0),
        }];
        let mut arms = [None; MAX_CATCH_ARMS];
        arms[0] = Some(CatchArm {
            variant: 0,
            handler_pc: 5,
        });
        let try_handlers = [TryHandlerEntry { arms }];
        let program = [
            Instr::EnterTry(0),               // 0
            Instr::Call(0),                   // 1: never runs risky's body
            Instr::Pop,                       // 2
            Instr::PushConst(Value::Int(0)),  // 3
            Instr::Jump(7),                   // 4: skip catch arm on normal completion
            Instr::PushConst(Value::Int(-1)), // 5: catch Boom
            Instr::Return,                    // 6
            Instr::ExitTry,                   // 7
            Instr::Return,                    // 8
        ];
        let mut vm = Vm::<8, 4, 2, 1>::new();
        assert_eq!(
            vm.run_with_tries(&functions, &try_handlers, &program),
            Ok(Value::Int(-1))
        );
    }

    #[test]
    fn call_inside_try_with_no_matching_arm_is_uncaught_checked_failure() {
        let risky = [Instr::Return];
        let functions = [FunctionDef {
            code: &risky,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: Some(7),
        }];
        let mut arms = [None; MAX_CATCH_ARMS];
        arms[0] = Some(CatchArm {
            variant: 1, // doesn't match variant 7
            handler_pc: 0,
        });
        let try_handlers = [TryHandlerEntry { arms }];
        let program = [
            Instr::EnterTry(0),
            Instr::Call(0),
            Instr::ExitTry,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2, 1>::new();
        assert_eq!(
            vm.run_with_tries(&functions, &try_handlers, &program),
            Err(VmError::CheckedFailure(7))
        );
    }

    #[test]
    fn call_inside_try_with_no_fails_variant_runs_normally() {
        // A try block with no matching catch is irrelevant to a
        // function that doesn't declare `fails` at all.
        let safe = [Instr::PushConst(Value::Int(9)), Instr::Return];
        let functions = [FunctionDef {
            code: &safe,
            arity: 0,
            local_count: 0,
            postcheck: None,
            fails_variant: None,
        }];
        let try_handlers = [TryHandlerEntry::EMPTY];
        let program = [
            Instr::EnterTry(0),
            Instr::Call(0),
            Instr::ExitTry,
            Instr::Return,
        ];
        let mut vm = Vm::<8, 4, 2, 1>::new();
        assert_eq!(
            vm.run_with_tries(&functions, &try_handlers, &program),
            Ok(Value::Int(9))
        );
    }

    #[test]
    fn checked_failure_unwinds_across_nested_call_frames() {
        // main -> outer() -> inner() (fails Boom), with the `try`
        // wrapping only the call to `outer`. The catch dispatch must
        // unwind both the `inner` and `outer` frames back to main.
        let inner = [Instr::Return];
        let outer = [
            Instr::Call(0), // call inner() — this is what raises Boom
            Instr::Return,
        ];
        let functions = [
            FunctionDef {
                code: &inner,
                arity: 0,
                local_count: 0,
                postcheck: None,
                fails_variant: Some(0),
            },
            FunctionDef {
                code: &outer,
                arity: 0,
                local_count: 0,
                postcheck: None,
                fails_variant: None,
            },
        ];
        let mut arms = [None; MAX_CATCH_ARMS];
        arms[0] = Some(CatchArm {
            variant: 0,
            handler_pc: 4,
        });
        let try_handlers = [TryHandlerEntry { arms }];
        let program = [
            Instr::EnterTry(0),               // 0
            Instr::Call(1),                   // 1: outer() -> inner() raises Boom
            Instr::Jump(6),                   // 2
            Instr::PushConst(Value::Int(0)),  // 3 (unreached)
            Instr::PushConst(Value::Int(-1)), // 4: catch Boom
            Instr::Return,                    // 5
            Instr::ExitTry,                   // 6
            Instr::Return,                    // 7
        ];
        let mut vm = Vm::<8, 4, 3, 1>::new();
        assert_eq!(
            vm.run_with_tries(&functions, &try_handlers, &program),
            Ok(Value::Int(-1))
        );
    }

    #[test]
    fn enter_try_beyond_tries_capacity_is_typed_error_not_a_panic() {
        let program = [
            Instr::EnterTry(0),
            Instr::EnterTry(0),
            Instr::PushConst(Value::Int(1)),
            Instr::Return,
        ];
        let try_handlers = [TryHandlerEntry::EMPTY];
        let mut vm = Vm::<8, 0, 1, 1>::new();
        assert_eq!(
            vm.run_with_tries(&[], &try_handlers, &program),
            Err(VmError::TryStackOverflow)
        );
    }

    #[test]
    fn enter_try_out_of_range_handler_is_typed_error_not_a_panic() {
        let program = [Instr::EnterTry(5), Instr::Return];
        let mut vm = Vm::<8, 0, 1, 1>::new();
        assert_eq!(
            vm.run_with_tries(&[], &[], &program),
            Err(VmError::TryHandlerOutOfBounds(5))
        );
    }

    #[test]
    fn exit_try_on_empty_try_stack_is_tolerated_not_an_error() {
        let program = [
            Instr::ExitTry,
            Instr::PushConst(Value::Int(1)),
            Instr::Return,
        ];
        let mut vm = Vm::<8, 0>::new();
        assert_eq!(vm.run(&program), Ok(Value::Int(1)));
    }
}
