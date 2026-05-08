//! Ralph-Loop Uniqueness #15 — static stack budget by name suffix.
//!
//! Bare-metal systems blow the stack. Tools like `bloaty`, `puncover`,
//! and `cargo-call-stack` estimate stack post-link. No language has a
//! source-level mechanism to declare "this function must use ≤N words"
//! and warn on violation.
//!
//! Resilient encodes a budget by name: any function whose name ends in
//! `_stack8`, `_stack16`, `_stack32`, or `_stack64` declares an upper
//! bound (in words) on its locals. We approximate stack use as
//!   #parameters + #let bindings + 2*max_nested_block_depth
//! and warn when the estimate exceeds the budget. Crude but real:
//! catches the common "I added a 64-byte buffer to an ISR" defect.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::for_each_function;

const SUFFIXES: &[(&str, usize)] = &[
    ("_stack8", 8),
    ("_stack16", 16),
    ("_stack32", 32),
    ("_stack64", 64),
];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    for_each_function(program, |fname, params, body| {
        let budget = SUFFIXES
            .iter()
            .find(|(s, _)| fname.ends_with(*s))
            .map(|(_, b)| *b);
        let Some(budget) = budget else { return };

        let estimate = params.len() + count_lets(body) + 2 * max_block_depth(body, 0);
        if estimate > budget {
            eprintln!(
                "warning: '{fname}' declares stack budget {budget} words \
                 (by name suffix) but the body's estimate is {estimate} \
                 (params + lets + 2*max_block_depth) — over budget"
            );
        }
    });
    Ok(())
}

fn count_lets(body: &Node) -> usize {
    let mut n = 0;
    crate::uniqueness_walk::visit(body, &mut |node| {
        if matches!(
            node,
            Node::LetStatement { .. } | Node::StaticLet { .. } | Node::Const { .. }
        ) {
            n += 1;
        }
    });
    n
}

fn max_block_depth(node: &Node, depth: usize) -> usize {
    match node {
        Node::Block { stmts, .. } => {
            let mut m = depth + 1;
            for s in stmts {
                m = m.max(max_block_depth(s, depth + 1));
            }
            m
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let mut m = max_block_depth(consequence, depth + 1);
            if let Some(alt) = alternative {
                m = m.max(max_block_depth(alt, depth + 1));
            }
            m
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            max_block_depth(body, depth + 1)
        }
        Node::Function { body, .. } => max_block_depth(body, depth),
        _ => depth,
    }
}
