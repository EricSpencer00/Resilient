//! Preconditions execute in the callee frame, after argument/upvalue binding.
//! Keeping them in the entry chunk also covers indirect and tail calls.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_requires_entry(
    name: &str,
    requires: &[Node],
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    fallback_line: u32,
) -> Result<(), CompileError> {
    if requires.is_empty() {
        return Ok(());
    }
    let name_const = chunk.add_string_constant(name)?;
    for clause in requires {
        let line = node_line(clause).unwrap_or(fallback_line);
        compile_expr(
            clause,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            line,
        )?;
        // JumpIfTrue consumes the predicate using the VM's normal truthiness.
        // Expression faults propagate unchanged; only a false predicate reaches
        // RequiresViolation. Later clauses and the body have not run yet.
        let success = chunk.emit(Op::JumpIfTrue(0), line);
        let clause_const = chunk.add_string_constant(&crate::format_contract_expr(clause))?;
        chunk.emit(
            Op::RequiresViolation {
                name_const,
                clause_const,
            },
            line,
        );
        chunk.patch_jump(success, chunk.code.len())?;
    }
    Ok(())
}
