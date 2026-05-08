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
    let mut decls: Vec<String> = Vec::new();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for stmt in stmts {
        if let Node::Function { name, body, .. } = &stmt.node {
            decls.push(name.clone());
            visit(body, &mut |n| {
                if let Node::CallExpression { function, .. } = n {
                    if let Node::Identifier { name: callee, .. } = function.as_ref() {
                        *counts.entry(callee.clone()).or_default() += 1;
                    }
                }
            });
        }
    }
    for d in &decls {
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
