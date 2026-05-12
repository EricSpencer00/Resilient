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
    // RES-1222: fast-reject — see stack_budget for the same pattern.
    // Skip the closure dispatch for programs that declare no
    // `_alloc{N}` suffix.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_budget = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { name, .. } if parse_budget(name).is_some()));
    if !has_budget {
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
