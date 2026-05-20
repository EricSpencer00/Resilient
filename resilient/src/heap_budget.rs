//! Ralph-Loop Uniqueness #16 — static heap-allocation budget per function.
//!
//! C/C++ have RAII; Rust has the "no_std + no_alloc" feature gate. Neither
//! lets you declare "this fn allocates at most N times" at the source.
//!
//! Resilient enforces an allocation budget by name suffix:
//!   `_alloc0` — must contain zero allocation calls
//!   `_alloc1`, `_alloc2`, …, `_alloc8` — at most that many call sites
//! Allocation is detected by call name: `Box`, `box_new`, `vec_new`,
//! `string_new`, `array_new`, `Vec`, `String`, `array`, `push`, `clone`,
//! and any free fn ending in `_alloc`.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{for_each_function, visit};

const ALLOC_FNS: &[&str] = &[
    "Box",
    "box_new",
    "vec_new",
    "string_new",
    "array_new",
    "Vec",
    "String",
    "array",
    "push",
    "clone",
    "concat",
];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1222 / RES-2304: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind
    // `markers.any_fn_name_with_suffix(&["_alloc0", "_alloc1",
    // "_alloc2", "_alloc3", "_alloc4", "_alloc5"])`, so the program
    // is guaranteed to contain at least one `_alloc{N}`-suffixed
    // function. The previous internal `stmts.iter().any(...)`
    // pre-scan walked the full top-level statement list a second
    // time for the same signal Markers already computed. Mirrors
    // RES-2292 / RES-2294 / RES-2296 / RES-2298 / RES-2300 / RES-2302.
    if !matches!(program, Node::Program(_)) {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        let Some(budget) = parse_budget(fname) else {
            return;
        };
        let allocs = count_allocs(body);
        if allocs > budget {
            eprintln!(
                "warning: '{fname}' declares heap budget {budget} \
                 (by name suffix) but the body contains {allocs} \
                 allocation call site(s) — exceeds budget"
            );
        }
    });
    Ok(())
}

// RES-1554: static `_alloc{N}` suffix table — the previous shape
// allocated 9 `String`s per `parse_budget` call via `format!`. The
// fast-reject `has_budget` check (scans every top-level fn) and the
// `for_each_function` enforcement loop both call it, so allocation
// volume scaled as 9·N for an N-function program with zero budgets.
const ALLOC_BUDGET_SUFFIXES: &[(&str, usize)] = &[
    ("_alloc0", 0),
    ("_alloc1", 1),
    ("_alloc2", 2),
    ("_alloc3", 3),
    ("_alloc4", 4),
    ("_alloc5", 5),
    ("_alloc6", 6),
    ("_alloc7", 7),
    ("_alloc8", 8),
];

fn parse_budget(name: &str) -> Option<usize> {
    ALLOC_BUDGET_SUFFIXES
        .iter()
        .find(|(suf, _)| name.ends_with(suf))
        .map(|(_, n)| *n)
}

fn count_allocs(body: &Node) -> usize {
    let mut n = 0;
    visit(body, &mut |node| {
        if let Node::CallExpression { function, .. } = node {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if ALLOC_FNS.contains(&name.as_str()) || name.ends_with("_alloc") {
                    n += 1;
                }
            }
        }
    });
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_trigger_returns_ok() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
