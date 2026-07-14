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
//! No heap, no `Vec`, no function-call stack (calls are out of
//! scope for this increment — see the design doc's opcode audit;
//! `Call`/`TailCall`/`ReturnFromCall` need a bounded call-frame
//! stack that a follow-up PR adds once this skeleton lands). The
//! operand stack and the locals slab are both fixed-capacity
//! arrays sized by `const` generics, mirroring the
//! `[TimerState; MAX_TIMERS]` fixed-array idiom already used by
//! [`crate::timer`] and the `Fixed<N, D>` const-generic idiom used
//! by [`crate::fixed`]:
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
}

/// A bytecode VM instance with a fixed-capacity operand stack
/// (`STACK` slots) and a fixed-capacity local-variable slab
/// (`LOCALS` slots). Both bounds are compile-time `const` generic
/// parameters — no heap, no growth, overflow is a typed
/// [`VmError`] rather than a panic.
///
/// Function calls (and therefore a call-frame stack) are out of
/// scope for this increment; `run` executes a single flat
/// instruction slice from index 0 until `Return` or an error.
pub struct Vm<const STACK: usize, const LOCALS: usize> {
    stack: [Value; STACK],
    sp: usize,
    locals: [Value; LOCALS],
}

impl<const STACK: usize, const LOCALS: usize> Default for Vm<STACK, LOCALS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const STACK: usize, const LOCALS: usize> Vm<STACK, LOCALS> {
    /// A fresh VM: empty operand stack, locals zero-initialised to
    /// `Value::Int(0)`.
    pub fn new() -> Self {
        Self {
            stack: [Value::Int(0); STACK],
            sp: 0,
            locals: [Value::Int(0); LOCALS],
        }
    }

    /// Overwrite the locals slab before a run (e.g. to seed
    /// function arguments). Returns `LocalsOutOfBounds` if `idx >=
    /// LOCALS` instead of panicking.
    pub fn set_local(&mut self, idx: u16, value: Value) -> Result<(), VmError> {
        match self.locals.get_mut(idx as usize) {
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
    /// first error. Every branch, arithmetic op, and stack/locals
    /// access is bounds-checked — no panic path exists here.
    pub fn run(&mut self, program: &[Instr]) -> Result<Value, VmError> {
        let mut pc: usize = 0;
        loop {
            let instr = *program.get(pc).ok_or(VmError::PcOutOfBounds)?;
            pc += 1;
            match instr {
                Instr::PushConst(v) => self.push(v)?,
                Instr::LoadLocal(idx) => {
                    let v = *self
                        .locals
                        .get(idx as usize)
                        .ok_or(VmError::LocalsOutOfBounds)?;
                    self.push(v)?;
                }
                Instr::StoreLocal(idx) => {
                    let v = self.pop()?;
                    let slot = self
                        .locals
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
                    pc = Self::validate_target(target, program.len())?;
                }
                Instr::JumpIfFalse(target) => {
                    let cond = self.pop()?.as_bool()?;
                    if !cond {
                        pc = Self::validate_target(target, program.len())?;
                    }
                }
                Instr::JumpIfTrue(target) => {
                    let cond = self.pop()?.as_bool()?;
                    if cond {
                        pc = Self::validate_target(target, program.len())?;
                    }
                }
                Instr::Return => return self.pop(),
            }
        }
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
}
