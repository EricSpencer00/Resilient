//! Loop guards share interpreter clause order and diagnostic locations.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn compile_loop_invariants(
    invariants: &[Node],
    body: &Node,
    loop_span: &crate::span::Span,
    chunk: &mut Chunk,
    locals: &mut HashMap<String, u16>,
    next_local: &mut u16,
    fn_index: &HashMap<String, u16>,
    ffi_index: &HashMap<String, u16>,
    fns: &mut Vec<Function>,
    next_fn_idx: &mut u16,
    fallback_line: u32,
) -> Result<(), CompileError> {
    let body_invariants = crate::loop_invariants::collect_body_invariants(body);
    for clause in invariants.iter().chain(body_invariants) {
        let span = crate::loop_invariants::node_span(clause).unwrap_or(loop_span);
        let message = format!(
            "runtime error: loop invariant violated at {}:{}",
            span.start.line, span.start.column
        );
        compile_assert_like(
            clause,
            &None,
            Op::AssertFail,
            &message,
            chunk,
            locals,
            next_local,
            fn_index,
            ffi_index,
            fns,
            next_fn_idx,
            node_line(clause).unwrap_or(fallback_line),
        )?;
    }
    Ok(())
}
