//! Ralph-Loop Uniqueness #14 — static rate-limit fan-in.
//!
//! Web frameworks rate-limit at runtime via middleware. Some embedded
//! schedulers (FreeRTOS, Zephyr) verify periodicity of tasks. No
//! language *statically* checks the call-site cardinality of a function
//! that should only be invoked from a small number of places.
//!
//! Resilient enforces a static-call-site bound by name suffix: any
//! function whose name ends with `_oncepertick` or `_singleshot` may be
//! called from at most one site in the entire program; functions
//! ending with `_few` may have at most three call sites. Anything more
//! is a warning.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::visit;
use std::collections::HashMap;

const ONCE_SUFFIXES: &[&str] = &["_oncepertick", "_singleshot"];
const FEW_SUFFIX: &str = "_few";

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1228 / RES-2300: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind
    // `markers.any_fn_name_with_suffix(&["_oncepertick",
    // "_singleshot", "_few"])`, so the program is guaranteed to
    // contain at least one rate-limited-suffixed function. The
    // previous internal `stmts.iter().any(...)` pre-scan walked the
    // full top-level statement list a second time for the same
    // signal Markers already computed during the shared whole-AST
    // walk. Mirrors RES-1916 / RES-1917 / RES-2292 / RES-2294 /
    // RES-2296 / RES-2298.
    // RES-1515: borrow the top-level fn names into `decls` as `&str`
    // instead of cloning. The `counts` HashMap still needs owned
    // String keys because `uniqueness_walk::visit` uses a HRTB
    // closure that can't bind the outer AST lifetime (same
    // limitation hit by RES-1509 / RES-1511). For the counts
    // map, apply the RES-1505 entry-clone gate: only allocate
    // when the callee is new, falling back to `get_mut` for repeat
    // hits. Mirrors RES-1495 / RES-1500 / RES-1503 / RES-1507 /
    // RES-1508 / RES-1509 / RES-1511.
    // RES-1788: pre-size decls to stmts.len() (every top-level
    // statement could be a Function, one push each).
    let mut decls: Vec<&str> = Vec::with_capacity(stmts.len());
    // RES-1962: pre-size to 8 — distinct callee counts in real
    // programs are typically 5-20, dominated by stdlib + user-fn
    // names. Skips the 0-bucket initial rehash for the common case.
    let mut counts: HashMap<String, usize> = HashMap::with_capacity(8);
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            decls.push(name.as_str());
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n {
                    if let Node::Identifier { name: callee, .. } = function.as_ref() {
                        if let Some(c) = counts.get_mut(callee.as_str()) {
                            *c += 1;
                        } else {
                            counts.insert(callee.clone(), 1);
                        }
                    }
                }
            });
        }
    }
    for &d in &decls {
        let limit = if ONCE_SUFFIXES.iter().any(|s| d.ends_with(*s)) {
            Some(1)
        } else if d.ends_with(FEW_SUFFIX) {
            Some(3)
        } else {
            None
        };
        if let Some(max) = limit {
            let c = counts.get(d).copied().unwrap_or(0);
            if c > max {
                eprintln!(
                    "warning: '{d}' is rate-limited (suffix says max {max} call site(s)) \
                     but the program contains {c} call sites — exceeds static budget"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_limited_fn_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn once_suffixes_include_oncepertick() {
        assert!(ONCE_SUFFIXES.contains(&"_oncepertick"));
    }
}
