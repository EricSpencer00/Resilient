//! RES-076: stack-based bytecode VM.
//!
//! Walks a `Chunk` produced by `compiler::compile`. The execution
//! model is dead simple: an operand stack of `Value`s, a locals
//! slab indexed by `u16`, and a single program counter into
//! `chunk.code`. There are no jumps yet (control-flow ops land in
//! RES-083), so `pc` only ever advances by 1.

#![allow(dead_code)]

use crate::bytecode::{Chunk, Op};
use crate::Value;

/// Errors the VM can surface at runtime. Like `CompileError`, the
/// `&'static str` payloads describe the offending construct without
/// allocating per-error.
#[derive(Debug, Clone, PartialEq)]
pub enum VmError {
    EmptyStack,
    DivideByZero,
    TypeMismatch(&'static str),
    LocalOutOfBounds(u16),
    ConstantOutOfBounds(u16),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::EmptyStack => write!(f, "vm: operand stack underflow"),
            VmError::DivideByZero => write!(f, "vm: divide by zero"),
            VmError::TypeMismatch(what) => write!(f, "vm: type mismatch in {}", what),
            VmError::LocalOutOfBounds(i) => write!(f, "vm: local {} out of bounds", i),
            VmError::ConstantOutOfBounds(i) => write!(f, "vm: constant {} out of bounds", i),
        }
    }
}

impl std::error::Error for VmError {}

/// Run a compiled chunk. Returns the value on top of the operand
/// stack at the moment `Op::Return` fires; `Value::Void` if the
/// stack was empty.
pub fn run(chunk: &Chunk) -> Result<Value, VmError> {
    let mut stack: Vec<Value> = Vec::with_capacity(64);
    // Locals slab is sized lazily — we grow on first store. The
    // compiler caps locals at u16 so this is bounded.
    let mut locals: Vec<Value> = Vec::new();

    let mut pc = 0usize;
    while pc < chunk.code.len() {
        let op = chunk.code[pc];
        pc += 1;
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
                let v = locals
                    .get(idx as usize)
                    .ok_or(VmError::LocalOutOfBounds(idx))?
                    .clone();
                stack.push(v);
            }
            Op::StoreLocal(idx) => {
                let v = stack.pop().ok_or(VmError::EmptyStack)?;
                let needed = (idx as usize) + 1;
                if locals.len() < needed {
                    locals.resize(needed, Value::Void);
                }
                locals[idx as usize] = v;
            }
            Op::Return => {
                return Ok(stack.pop().unwrap_or(Value::Void));
            }
        }
    }
    // Program ran off the end without an explicit Return — same as
    // returning Void from the trailing implicit Return that the
    // compiler always appends.
    Ok(stack.pop().unwrap_or(Value::Void))
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
    use crate::bytecode::Op;

    fn const_chunk(values: &[Value], code: &[Op]) -> Chunk {
        let mut c = Chunk::new();
        for v in values {
            c.constants.push(v.clone());
        }
        for op in code {
            c.code.push(*op);
            c.line_info.push(1);
        }
        c
    }

    /// Test helper: run + assert the result is `Value::Int(expected)`.
    /// `Value` doesn't implement `PartialEq` (it carries a `Function`
    /// variant whose body is `Box<Node>`, which would require a deep
    /// equality not worth the maintenance burden), so the VM tests
    /// destructure manually.
    fn assert_int(actual: Value, expected: i64) {
        match actual {
            Value::Int(v) => assert_eq!(v, expected, "expected Int({}), got Int({})", expected, v),
            other => panic!("expected Int({}), got {:?}", expected, other),
        }
    }

    #[test]
    fn const_then_return_yields_value() {
        let c = const_chunk(&[Value::Int(7)], &[Op::Const(0), Op::Return]);
        assert_int(run(&c).unwrap(), 7);
    }

    #[test]
    fn add_two_ints() {
        // 2 + 3 = 5
        let c = const_chunk(
            &[Value::Int(2), Value::Int(3)],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        assert_int(run(&c).unwrap(), 5);
    }

    #[test]
    fn end_to_end_two_plus_three_times_four() {
        // 2 + 3 * 4 = 14, compiled by the real compiler
        let (program, _) = crate::parse("2 + 3 * 4;");
        let chunk = crate::compiler::compile(&program).unwrap();
        assert_int(run(&chunk).unwrap(), 14);
    }

    #[test]
    fn let_then_load_yields_stored_value() {
        // let x = 9; x;
        let (program, _) = crate::parse("let x = 9; x;");
        let chunk = crate::compiler::compile(&program).unwrap();
        assert_int(run(&chunk).unwrap(), 9);
    }

    #[test]
    fn divide_by_zero_is_clean_error() {
        let (program, _) = crate::parse("10 / 0;");
        let chunk = crate::compiler::compile(&program).unwrap();
        let err = run(&chunk).unwrap_err();
        assert_eq!(err, VmError::DivideByZero);
    }

    #[test]
    fn type_mismatch_on_add_with_string_constant() {
        // Hand-roll a chunk that tries to Add an Int to a String —
        // the compiler won't emit this directly, but the VM must
        // refuse cleanly anyway.
        let c = const_chunk(
            &[Value::Int(1), Value::String("x".into())],
            &[Op::Const(0), Op::Const(1), Op::Add, Op::Return],
        );
        assert_eq!(run(&c).unwrap_err(), VmError::TypeMismatch("Add"));
    }

    #[test]
    fn negation_works() {
        let (program, _) = crate::parse("let x = -7; x;");
        let chunk = crate::compiler::compile(&program).unwrap();
        assert_int(run(&chunk).unwrap(), -7);
    }
}
