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
/// write the locals slab indexed by `u16`.
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
    /// Push `locals[idx]` onto the operand stack.
    LoadLocal(u16),
    /// Pop the operand stack and store into `locals[idx]`.
    StoreLocal(u16),
    /// Halt execution. The top of the operand stack (if any) is the
    /// program's return value; an empty stack returns `Value::Void`.
    Return,
}

/// One compiled program. `code` is the instruction stream;
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
}
