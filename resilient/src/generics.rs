/// RES-289 (RES-81): generic type-parameter validation pass.
///
/// The interpreter is dynamically typed, so no monomorphisation is
/// performed here. This pass only checks that the type-parameter list
/// on each `fn<T, U> name(...)` declaration is syntactically valid and
/// contains no duplicate names. Semantic checking (substitution,
/// instantiation) is deferred to a future ticket.
use crate::Node;

/// Walk the top-level program and validate all generic function
/// declarations.  Returns `Err` with a diagnostic string on the first
/// violation.
pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let stmts = match program {
        Node::Program(stmts) => stmts,
        _ => return Ok(()),
    };
    for stmt in stmts {
        check_node(&stmt.node)?;
    }
    Ok(())
}

fn check_node(node: &Node) -> Result<(), String> {
    if let Node::Function {
        name, type_params, ..
    } = node
    {
        let mut seen = std::collections::HashSet::new();
        for tp in type_params {
            if !seen.insert(tp.as_str()) {
                return Err(format!(
                    "duplicate type parameter `{}` in function `{}`",
                    tp, name
                ));
            }
        }
    }
    Ok(())
}
