//! RES-173: human-readable VM bytecode disassembler.
//!
//! Entry: `disassemble(program: &Program, out: &mut impl Write)`
//! prints every chunk in a stable, parseable line format that
//! external tools can consume.
//!
//! ## Output format (STABLE — external tools parse this)
//!
//! The driver writes one "section" per chunk — `main` first, then
//! each function in the order `Program::functions` stores them.
//! A section is a header line plus an indented body:
//!
//! ```text
//! === main ===
//! constants:
//!   const[0] = 7
//!   const[1] = "hello"
//! code:
//!   0000  L5   Const 0
//!   0001  L5   Return
//! ```
//!
//! Per-op lines use four columns separated by whitespace:
//!
//! 1. **offset** — zero-padded 4-digit hex PC, e.g. `000A`.
//! 2. **line** — `L<n>` where `n` is the 1-indexed source line
//!    from `Chunk::line_info` (respects RES-091). Prints `L0`
//!    for synthetic instructions (line info is 0 for those).
//! 3. **OpName** — exact variant name (`Const`, `LoadLocal`, …).
//! 4. **operands** — zero or more whitespace-separated tokens,
//!    see per-op format below.
//!
//! Operand rendering by op:
//!
//! | Op                 | Operands rendered as                          |
//! | ------------------ | --------------------------------------------- |
//! | `Const(i)`         | `i` plus `; const[i] = <Value>` comment       |
//! | `LoadLocal(i)`     | `i`                                           |
//! | `StoreLocal(i)`    | `i`                                           |
//! | `IncLocal(i)`      | `i`                                           |
//! | `Call(i)`          | `i` plus `; -> <fn-name>` comment             |
//! | `Jump(o)`          | `-> 0000` absolute-target (4-hex, after ":")  |
//! | `JumpIfFalse(o)`   | `-> 0000`                                     |
//! | `JumpIfTrue(o)`    | `-> 0000`                                     |
//! | everything else    | (no operands)                                 |
//!
//! Peephole (RES-172) effects are reflected automatically because
//! the compiler runs peephole BEFORE this disassembler sees the
//! chunks — the ticket's "reflect the optimized bytecode" rule.

use std::fmt::Write;

use crate::bytecode::{Chunk, Op, Program};

/// RES-173: disassemble the whole program to a writer. Returns
/// `Ok(())` on success; I/O failures bubble up as `fmt::Error`.
pub fn disassemble(program: &Program, out: &mut String) -> std::fmt::Result {
    // Build a function-index → name map so `Call(i)` operands can
    // include a readable comment.
    let fn_names: Vec<&str> = program.functions.iter().map(|f| f.name.as_str()).collect();

    // Main chunk first.
    writeln!(out, "=== main ===")?;
    disassemble_chunk(&program.main, &fn_names, out)?;

    // Each user function, in declaration order.
    for func in &program.functions {
        writeln!(
            out,
            "=== fn {} (arity={}, locals={}) ===",
            func.name, func.arity, func.local_count
        )?;
        disassemble_chunk(&func.chunk, &fn_names, out)?;
    }
    Ok(())
}

/// Disassemble a single chunk: constants block, then code block.
/// Chunk-scoped — `fn_names` passed in for `Call` comment rendering.
fn disassemble_chunk(chunk: &Chunk, fn_names: &[&str], out: &mut String) -> std::fmt::Result {
    // Constants section.
    if chunk.constants.is_empty() {
        writeln!(out, "constants: (none)")?;
    } else {
        writeln!(out, "constants:")?;
        for (i, v) in chunk.constants.iter().enumerate() {
            writeln!(out, "  const[{}] = {}", i, v)?;
        }
    }

    // Code section.
    writeln!(out, "code:")?;
    if chunk.code.is_empty() {
        writeln!(out, "  (empty)")?;
        return Ok(());
    }
    for (pc, op) in chunk.code.iter().enumerate() {
        let line = chunk.line_info.get(pc).copied().unwrap_or(0);
        let line_col = format!("L{}", line);
        // Ticket format: `<offset:04x>  <line>   <OpName> <operands>`
        write!(out, "  {:04x}  {:<5}", pc, line_col)?;
        write_op(op, pc, chunk, fn_names, out)?;
        writeln!(out)?;
    }
    Ok(())
}

/// Render a single op onto `out` with its operands. No newline
/// emitted; caller adds the line terminator.
fn write_op(
    op: &Op,
    pc: usize,
    chunk: &Chunk,
    fn_names: &[&str],
    out: &mut String,
) -> std::fmt::Result {
    match *op {
        Op::Const(idx) => {
            write!(out, "Const {}", idx)?;
            if let Some(v) = chunk.constants.get(idx as usize) {
                write!(out, "      ; const[{}] = {}", idx, v)?;
            }
        }
        Op::Add => write!(out, "Add")?,
        Op::Sub => write!(out, "Sub")?,
        Op::Mul => write!(out, "Mul")?,
        Op::Div => write!(out, "Div")?,
        Op::Mod => write!(out, "Mod")?,
        Op::Neg => write!(out, "Neg")?,
        Op::LoadLocal(idx) => write!(out, "LoadLocal {}", idx)?,
        Op::StoreLocal(idx) => write!(out, "StoreLocal {}", idx)?,
        Op::IncLocal(idx) => write!(out, "IncLocal {}", idx)?,
        Op::Call(idx) => {
            write!(out, "Call {}", idx)?;
            if let Some(name) = fn_names.get(idx as usize) {
                write!(out, "          ; -> {}", name)?;
            }
        }
        Op::ReturnFromCall => write!(out, "ReturnFromCall")?,
        // RES-384: TailCall reuses the current frame (TCO).
        Op::TailCall(idx) => {
            write!(out, "TailCall {}", idx)?;
            if let Some(name) = fn_names.get(idx as usize) {
                write!(out, "       ; -> {} (tail)", name)?;
            }
        }
        Op::Jump(offset) => {
            let target = (pc as isize + 1) + offset as isize;
            write!(out, "Jump         -> {:04x}", target.max(0) as usize)?;
        }
        Op::JumpIfFalse(offset) => {
            let target = (pc as isize + 1) + offset as isize;
            write!(out, "JumpIfFalse  -> {:04x}", target.max(0) as usize)?;
        }
        Op::JumpIfTrue(offset) => {
            let target = (pc as isize + 1) + offset as isize;
            write!(out, "JumpIfTrue   -> {:04x}", target.max(0) as usize)?;
        }
        Op::Eq => write!(out, "Eq")?,
        Op::Neq => write!(out, "Neq")?,
        Op::Lt => write!(out, "Lt")?,
        Op::Le => write!(out, "Le")?,
        Op::Gt => write!(out, "Gt")?,
        Op::Ge => write!(out, "Ge")?,
        Op::Not => write!(out, "Not")?,
        Op::Return => write!(out, "Return")?,
        // RES-169a: skeleton disassembly for the two unused
        // closure opcodes. RES-169b/c will make these emittable
        // and executable; the format mirrors how the VM will
        // ultimately interpret the operands.
        Op::MakeClosure {
            fn_idx,
            upvalue_count,
        } => {
            write!(out, "MakeClosure {} {}", fn_idx, upvalue_count)?;
            if let Some(name) = fn_names.get(fn_idx as usize) {
                write!(out, "  ; -> {}", name)?;
            }
        }
        Op::LoadUpvalue(idx) => write!(out, "LoadUpvalue {}", idx)?,
        // RES-171a: array-ops disasm.
        Op::MakeArray { len } => write!(out, "MakeArray {}", len)?,
        Op::LoadIndex => write!(out, "LoadIndex")?,
        Op::StoreIndex => write!(out, "StoreIndex")?,
        // FFI v2.
        Op::CallForeign(idx) => write!(out, "CallForeign {idx:<6}")?,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use crate::bytecode::{Chunk, Function, Op, Program};

    fn mk_chunk(code: Vec<Op>, constants: Vec<Value>, lines: Vec<u32>) -> Chunk {
        Chunk {
            code,
            constants,
            line_info: lines,
        }
    }

    /// Build a Program with the given main chunk and no user fns.
    fn prog_main_only(main: Chunk) -> Program {
        Program {
            main,
            functions: Vec::new(),
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        }
    }

    #[test]
    fn disassemble_renders_constants_section() {
        let program = prog_main_only(mk_chunk(
            vec![Op::Const(0), Op::Return],
            vec![Value::Int(42)],
            vec![1, 1],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(out.contains("=== main ==="));
        assert!(out.contains("const[0] = 42"));
        assert!(out.contains("Const 0"));
    }

    #[test]
    fn disassemble_renders_jump_as_absolute_target() {
        // Jump(+1) at PC=0 lands at PC=2.
        let program = prog_main_only(mk_chunk(
            vec![Op::Jump(1), Op::Return, Op::Return],
            vec![],
            vec![3, 3, 3],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        // Absolute target rendered as 4-hex `-> 0002`.
        assert!(
            out.contains("-> 0002"),
            "expected `-> 0002` absolute target in disasm, got:\n{}",
            out
        );
    }

    #[test]
    fn disassemble_renders_line_column() {
        let program = prog_main_only(mk_chunk(
            vec![Op::Const(0), Op::Return],
            vec![Value::Int(7)],
            vec![42, 42],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        // Line column should show L42 for both ops.
        assert!(
            out.lines().filter(|l| l.contains("L42")).count() >= 2,
            "expected two L42 lines, got:\n{}",
            out
        );
    }

    #[test]
    fn disassemble_renders_call_with_function_name() {
        let program = Program {
            main: mk_chunk(vec![Op::Call(0), Op::Return], vec![], vec![5, 5]),
            functions: vec![Function {
                name: "my_fn".to_string(),
                arity: 0,
                chunk: mk_chunk(vec![Op::Return], vec![], vec![1]),
                local_count: 0,
            }],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(
            out.contains("Call 0") && out.contains("-> my_fn"),
            "expected `Call 0 ... -> my_fn`, got:\n{}",
            out
        );
    }

    #[test]
    fn disassemble_includes_each_function_header() {
        let program = Program {
            main: mk_chunk(vec![Op::Return], vec![], vec![1]),
            functions: vec![
                Function {
                    name: "alpha".to_string(),
                    arity: 1,
                    chunk: mk_chunk(
                        vec![Op::LoadLocal(0), Op::ReturnFromCall],
                        vec![],
                        vec![1, 1],
                    ),
                    local_count: 1,
                },
                Function {
                    name: "beta".to_string(),
                    arity: 2,
                    chunk: mk_chunk(vec![Op::Return], vec![], vec![1]),
                    local_count: 2,
                },
            ],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(out.contains("=== fn alpha (arity=1, locals=1) ==="));
        assert!(out.contains("=== fn beta (arity=2, locals=2) ==="));
    }

    #[test]
    fn disassemble_covers_inclocal_and_jumpiftrue_after_peephole() {
        // Both ops are RES-172 additions — the disassembler must
        // render them with their operand.
        let program = prog_main_only(mk_chunk(
            vec![Op::IncLocal(2), Op::JumpIfTrue(0), Op::Return],
            vec![],
            vec![1, 1, 1],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(out.contains("IncLocal 2"), "disasm:\n{}", out);
        assert!(out.contains("JumpIfTrue"), "disasm:\n{}", out);
    }

    #[test]
    fn empty_chunk_prints_empty_marker() {
        let program = prog_main_only(mk_chunk(vec![], vec![], vec![]));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(out.contains("(empty)"));
        assert!(out.contains("constants: (none)"));
    }

    // ---------- RES-384: TailCall disasm ----------

    #[test]
    fn res384_tail_call_renders_with_fn_name() {
        let program = Program {
            main: mk_chunk(vec![Op::Return], vec![], vec![1]),
            functions: vec![Function {
                name: "loop_fn".to_string(),
                arity: 1,
                chunk: mk_chunk(
                    vec![
                        Op::TailCall(0),
                        Op::Return, // unreachable tombstone
                    ],
                    vec![],
                    vec![1, 1],
                ),
                local_count: 1,
            }],
        };
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(
            out.contains("TailCall 0") && out.contains("loop_fn"),
            "expected `TailCall 0 ... loop_fn` in disasm, got:\n{}",
            out
        );
        assert!(
            out.contains("(tail)"),
            "expected `(tail)` annotation in disasm, got:\n{}",
            out
        );
    }

    // ---------- RES-169a: skeleton closure-opcode disasm ----------

    #[test]
    fn res169a_make_closure_renders_with_operands() {
        let program = prog_main_only(mk_chunk(
            vec![
                Op::MakeClosure {
                    fn_idx: 0,
                    upvalue_count: 2,
                },
                Op::Return,
            ],
            vec![],
            vec![1, 1],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(
            out.contains("MakeClosure 0 2"),
            "expected `MakeClosure 0 2` in disasm, got:\n{}",
            out
        );
    }

    #[test]
    fn res169a_load_upvalue_renders_with_idx() {
        let program = prog_main_only(mk_chunk(
            vec![Op::LoadUpvalue(3), Op::Return],
            vec![],
            vec![1, 1],
        ));
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(
            out.contains("LoadUpvalue 3"),
            "expected `LoadUpvalue 3` in disasm, got:\n{}",
            out
        );
    }

    #[test]
    fn res169a_make_closure_with_named_fn_renders_pointer() {
        let program = Program {
            main: mk_chunk(
                vec![
                    Op::MakeClosure {
                        fn_idx: 0,
                        upvalue_count: 1,
                    },
                    Op::Return,
                ],
                vec![],
                vec![1, 1],
            ),
            functions: vec![Function {
                name: "adder".into(),
                arity: 1,
                chunk: Chunk::new(),
                local_count: 0,
            }],
            #[cfg(feature = "ffi")]
            foreign_syms: Vec::new(),
        };
        let mut out = String::new();
        disassemble(&program, &mut out).unwrap();
        assert!(
            out.contains("MakeClosure 0 1") && out.contains("-> adder"),
            "expected `MakeClosure 0 1 ... -> adder` in disasm, got:\n{}",
            out
        );
    }
}
