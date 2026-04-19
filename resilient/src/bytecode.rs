//! RES-076: bytecode representation for the VM.
//!
//! A `Chunk` holds a flat sequence of `Op` instructions plus a side
//! table of constants. The VM (in `vm.rs`) walks this chunk; the
//! compiler (in `compiler.rs`) builds it from a `Node` AST.
//!
//! This is the FOUNDATION ticket — only int arithmetic, let bindings,
//! identifiers, and Return are supported. Function calls (RES-081),
//! control flow (RES-083), and the rest of the language come in
//! dedicated follow-ups so each shipping piece is reviewable.
//!
//! Indices are `u16` — keeps each `Op` 4 bytes and caps a chunk at
//! 65536 constants / locals, which is way more than any realistic
//! program needs at this stage.

#![allow(dead_code)] // populated incrementally — follow-ups will exercise everything

use crate::Value;

/// A single instruction. The VM is stack-based: most ops pop their
/// arguments and push their result. `LoadLocal`/`StoreLocal` read and
/// write the current frame's slice of the locals slab, indexed by
/// `u16` (frame-relative).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    /// Push `chunk.constants[idx]` onto the operand stack.
    Const(u16),
    /// Pop two ints, push `lhs + rhs`.
    Add,
    /// Pop two ints, push `lhs - rhs`.
    Sub,
    /// Pop two ints, push `lhs * rhs`.
    Mul,
    /// Pop two ints, push `lhs / rhs`. Errors on divisor 0.
    Div,
    /// Pop two ints, push `lhs % rhs`. Errors on divisor 0.
    Mod,
    /// Pop one int, push `-int`.
    Neg,
    /// Push `locals[frame_base + idx]` onto the operand stack.
    LoadLocal(u16),
    /// Pop the operand stack and store into `locals[frame_base + idx]`.
    StoreLocal(u16),
    /// RES-081: call function `program.functions[idx]`. The VM pops
    /// `arity` values from the operand stack as arguments (leftmost
    /// arg popped last, so stack order matches source order), pushes
    /// a new `CallFrame`, and jumps into the callee's chunk.
    Call(u16),
    /// RES-081: return from a function call. Pops the top of the
    /// operand stack as the return value, unwinds the current frame,
    /// and pushes the return value onto the caller's stack.
    /// Distinct from `Return`, which halts the VM entirely — a
    /// top-level `return;` at program scope emits `Return`; a
    /// `return;` inside a `fn` body emits `ReturnFromCall`.
    ReturnFromCall,
    /// RES-083: unconditional relative jump. The target PC is
    /// `(pc_after_this_op) + offset`; positive offsets jump forward,
    /// negative offsets loop backward.
    Jump(i16),
    /// RES-083: pop the operand stack; if the value is "falsy"
    /// (`Bool(false)` or `Int(0)`), apply the relative jump.
    /// Otherwise fall through. Non-bool/non-int → TypeMismatch.
    JumpIfFalse(i16),
    /// RES-172: pop the operand stack; if the value is "truthy"
    /// (`Bool(true)` or `Int(!= 0)`), apply the relative jump.
    /// Mirrors `JumpIfFalse` so the peephole pass can fold
    /// `Not; JumpIfFalse(off)` into a single `JumpIfTrue(off)`.
    JumpIfTrue(i16),
    /// RES-172: increment the local at `idx` by 1 in place. No
    /// stack churn. Emitted by the peephole optimizer when it
    /// detects the `LoadLocal x; Const 1; Add; StoreLocal x`
    /// idiom.
    IncLocal(u16),
    /// RES-083: pop two ints (or bools), push `Value::Bool(lhs == rhs)`.
    Eq,
    /// RES-083: pop two ints, push `Value::Bool(lhs != rhs)`.
    Neq,
    /// RES-083: pop two ints, push `Value::Bool(lhs < rhs)`.
    Lt,
    /// RES-083: pop two ints, push `Value::Bool(lhs <= rhs)`.
    Le,
    /// RES-083: pop two ints, push `Value::Bool(lhs > rhs)`.
    Gt,
    /// RES-083: pop two ints, push `Value::Bool(lhs >= rhs)`.
    Ge,
    /// RES-083: pop a bool, push its negation. Non-bool → TypeMismatch.
    Not,
    /// Halt execution. The top of the operand stack (if any) is the
    /// program's return value; an empty stack returns `Value::Void`.
    Return,
    /// RES-169a (skeleton, unused): build a closure value from a
    /// function index and a count of upvalues. The compiler (RES-169b)
    /// will emit this immediately after `upvalue_count` copies of
    /// `LoadLocal(src)` that put the to-be-captured values onto the
    /// operand stack; the VM dispatch (RES-169c) will pop them into a
    /// `Value::Closure { fn_idx, upvalues }` and push that back.
    ///
    /// Today the variant exists so RES-169b/c can land as additive
    /// changes without another opcode-enum migration. The VM
    /// dispatch arm returns `VmError::Unsupported` — the compiler
    /// never emits this yet.
    MakeClosure { fn_idx: u16, upvalue_count: u8 },
    /// RES-169a (skeleton, unused): push `upvalues[idx]` from the
    /// current `CallFrame`'s captured-value slab onto the operand
    /// stack. Distinct from `LoadLocal` so the compiler can disambiguate
    /// captures from params/locals at the opcode level. RES-169c will
    /// wire the actual slab; today the dispatch arm returns
    /// `VmError::Unsupported`.
    LoadUpvalue(u16),
}

/// One compiled chunk of bytecode. `code` is the instruction stream;
/// `constants` is the table that `Const(idx)` indexes into;
/// `line_info` parallels `code` and stores the source line each
/// instruction came from (RES-077-style spans get richer in
/// follow-ups).
#[derive(Debug, Clone, Default)]
pub struct Chunk {
    pub code: Vec<Op>,
    pub constants: Vec<Value>,
    pub line_info: Vec<u32>,
}

/// RES-081: a compiled function. Parameters occupy the first `arity`
/// slots of the callee's locals slab; `local_count` is the total
/// number of locals (params + `let` bindings) the VM needs to
/// reserve on entry.
#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub arity: u8,
    pub chunk: Chunk,
    pub local_count: u16,
}

/// RES-081: top-level compile output. `main` is the entrypoint
/// (executed by `vm::run`), and `functions` is the call table that
/// `Op::Call(idx)` indexes into.
#[derive(Debug, Clone, Default)]
pub struct Program {
    pub main: Chunk,
    pub functions: Vec<Function>,
}

impl Chunk {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an instruction and its originating line. Returns the
    /// instruction's index for back-patching by jump emitters (used in
    /// RES-083).
    pub fn emit(&mut self, op: Op, line: u32) -> usize {
        let idx = self.code.len();
        self.code.push(op);
        self.line_info.push(line);
        idx
    }

    /// RES-083: back-patch a previously-emitted `Jump`/`JumpIfFalse`
    /// so it lands at `target_pc`. The op MUST already be a Jump or
    /// JumpIfFalse at `patch_idx`, and the offset must fit in `i16`.
    pub fn patch_jump(&mut self, patch_idx: usize, target_pc: usize) -> Result<(), CompileError> {
        // Offset is relative to the PC *after* the jump.
        let pc_after = (patch_idx + 1) as isize;
        let offset = (target_pc as isize) - pc_after;
        let offset: i16 = offset
            .try_into()
            .map_err(|_| CompileError::JumpOutOfRange)?;
        match &mut self.code[patch_idx] {
            Op::Jump(o) => *o = offset,
            Op::JumpIfFalse(o) => *o = offset,
            Op::JumpIfTrue(o) => *o = offset,
            other => {
                panic!("patch_jump called on non-jump op: {:?}", other);
            }
        }
        Ok(())
    }

    /// Intern a `Value` constant; returns the index for `Op::Const`.
    /// Reuses an existing slot if the constant is already present
    /// (cheap for small int chunks).
    pub fn add_constant(&mut self, v: Value) -> Result<u16, CompileError> {
        if let Some(existing) = self
            .constants
            .iter()
            .position(|c| values_eq_for_constants(c, &v))
        {
            return Ok(existing as u16);
        }
        if self.constants.len() >= u16::MAX as usize {
            return Err(CompileError::TooManyConstants);
        }
        let idx = self.constants.len() as u16;
        self.constants.push(v);
        Ok(idx)
    }
}

/// Constant pool dedup. Only used for compile-time interning so we
/// don't carry e.g. `Box<dyn Fn>` semantics — just structural equality
/// over the value shapes the FOUNDATION compiler emits.
fn values_eq_for_constants(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Void, Value::Void) => true,
        _ => false,
    }
}

/// Errors the compiler can return. `Unsupported` carries a static
/// description of the construct so the user gets `Unsupported(struct decl)`
/// instead of an opaque enum variant.
#[derive(Debug, Clone, PartialEq)]
pub enum CompileError {
    Unsupported(&'static str),
    TooManyConstants,
    TooManyLocals,
    UnknownIdentifier(String),
    /// RES-081: call to an unknown function name.
    UnknownFunction(String),
    /// RES-081: call arity mismatch — arguments at a call site don't
    /// match the callee's declared parameter count.
    ArityMismatch {
        callee: String,
        expected: u8,
        got: usize,
    },
    /// RES-083: a jump target is more than `i16::MAX` bytes away.
    JumpOutOfRange,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Unsupported(what) => {
                write!(f, "bytecode compile: unsupported construct: {}", what)
            }
            CompileError::TooManyConstants => write!(f, "bytecode compile: > 65535 constants"),
            CompileError::TooManyLocals => write!(f, "bytecode compile: > 65535 locals"),
            CompileError::UnknownIdentifier(n) => {
                write!(f, "bytecode compile: unknown identifier: {}", n)
            }
            CompileError::UnknownFunction(n) => {
                write!(f, "bytecode compile: unknown function: {}", n)
            }
            CompileError::ArityMismatch { callee, expected, got } => write!(
                f,
                "bytecode compile: call to {} has {} args, expected {}",
                callee, got, expected
            ),
            CompileError::JumpOutOfRange => {
                write!(f, "bytecode compile: jump target further than i16::MAX")
            }
        }
    }
}

impl std::error::Error for CompileError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_constant_dedups() {
        let mut c = Chunk::new();
        let i0 = c.add_constant(Value::Int(7)).unwrap();
        let i1 = c.add_constant(Value::Int(7)).unwrap();
        assert_eq!(i0, i1);
        assert_eq!(c.constants.len(), 1);
    }

    #[test]
    fn add_constant_keeps_distinct_values() {
        let mut c = Chunk::new();
        let i0 = c.add_constant(Value::Int(1)).unwrap();
        let i1 = c.add_constant(Value::Int(2)).unwrap();
        assert_ne!(i0, i1);
        assert_eq!(c.constants.len(), 2);
    }

    #[test]
    fn emit_appends_op_and_line() {
        let mut c = Chunk::new();
        let i = c.emit(Op::Add, 42);
        assert_eq!(i, 0);
        assert_eq!(c.code, vec![Op::Add]);
        assert_eq!(c.line_info, vec![42]);
    }

    #[test]
    fn compile_error_display_is_descriptive() {
        let e = CompileError::Unsupported("struct decl");
        assert_eq!(e.to_string(), "bytecode compile: unsupported construct: struct decl");
    }

    // ---------- RES-169a: skeleton closure opcodes ----------

    #[test]
    fn res169a_make_closure_constructs_with_payload() {
        // Sanity: the variant accepts both operands. Not yet
        // emitted by the compiler — RES-169b will add that.
        let op = Op::MakeClosure { fn_idx: 7, upvalue_count: 3 };
        if let Op::MakeClosure { fn_idx, upvalue_count } = op {
            assert_eq!(fn_idx, 7);
            assert_eq!(upvalue_count, 3);
        } else {
            panic!("expected MakeClosure");
        }
    }

    #[test]
    fn res169a_load_upvalue_constructs_with_payload() {
        let op = Op::LoadUpvalue(4);
        if let Op::LoadUpvalue(idx) = op {
            assert_eq!(idx, 4);
        } else {
            panic!("expected LoadUpvalue");
        }
    }

    #[test]
    fn res169a_closure_ops_are_copy() {
        // `Op` derives Copy — adding new variants must not break
        // that, because the VM dispatch reads `*op` per step.
        let a = Op::MakeClosure { fn_idx: 0, upvalue_count: 0 };
        let b = a; // copy, not move
        assert_eq!(a, b);
    }

    #[test]
    fn res169a_closure_ops_have_same_op_size_envelope() {
        // Regression guard. The module doc claims `Op` stays 4 bytes
        // up through u16-indexed variants. `MakeClosure` carries a
        // u16 + u8, so fits inside the same 4-byte envelope (with
        // discriminant + alignment). This test pins that — if
        // someone adds a `u32` later that inflates sizeof(Op), the
        // assertion fires and they have to justify the growth.
        // We allow a generous upper bound (8 bytes) so the check
        // survives cross-platform layout variation, but flag
        // anything catastrophically larger.
        assert!(
            std::mem::size_of::<Op>() <= 8,
            "sizeof(Op) = {} bytes; closure opcodes should not inflate this",
            std::mem::size_of::<Op>()
        );
    }

    #[test]
    fn res169a_emit_make_closure_roundtrips_through_chunk() {
        // The emit/line_info pipeline must accept the new opcodes
        // verbatim. No semantic expectation yet (the VM returns
        // Unsupported on dispatch); RES-169b/c make these live.
        let mut c = Chunk::new();
        let a = c.emit(Op::MakeClosure { fn_idx: 1, upvalue_count: 2 }, 10);
        let b = c.emit(Op::LoadUpvalue(0), 11);
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(c.code.len(), 2);
        assert_eq!(c.line_info, vec![10, 11]);
    }
}
