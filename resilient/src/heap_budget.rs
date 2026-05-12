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

fn parse_budget(name: &str) -> Option<usize> {
    for n in 0..=8 {
        let suf = format!("_alloc{n}");
        if name.ends_with(&suf) {
            return Some(n);
        }
    }
    None
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
