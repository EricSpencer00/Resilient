//! Ralph-Loop Uniqueness #22 — idempotency-required handlers.
//!
//! Distributed systems (Stripe, AWS, Kafka) document idempotency keys
//! at the API level. SQS / RabbitMQ deliver "at-least-once" — handlers
//! must be idempotent. No language enforces, at compile time, that a
//! handler advertised as idempotent actually performs an idempotency
//! check.
//!
//! Resilient flags any function whose name ends with `_idempotent` and
//! requires its body to either:
//!   * read from a store named with prefix `seen_` / `processed_`
//!     (FieldAccess), or
//!   * call a fn named `is_duplicate` / `was_seen` / `dedupe`.
//! Otherwise we warn that idempotency is claimed but not enforced.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function};

const DEDUPE_FNS: &[&str] = &["is_duplicate", "was_seen", "dedupe", "is_processed"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1217 / RES-2308: the typechecker's `<EXTENSION_PASSES>`
    // dispatch gates this call behind
    // `markers.any_fn_name_with_suffix(&["_idempotent"])`, so the
    // program is guaranteed to contain at least one
    // `_idempotent`-suffixed function. The previous internal
    // `stmts.iter().any(...)` pre-scan walked the full top-level
    // statement list a second time for the same signal Markers
    // already computed. Mirrors RES-2292 through RES-2306.
    if !matches!(program, Node::Program(_)) {
        return Ok(());
    }
    for_each_function(program, |fname, _params, body| {
        if !fname.ends_with("_idempotent") {
            return;
        }
        let ok = any_node(body, |n| match n {
            Node::CallExpression { function, .. } => match function.as_ref() {
                Node::Identifier { name, .. } => DEDUPE_FNS.contains(&name.as_str()),
                _ => false,
            },
            Node::FieldAccess { field, .. } => {
                field.starts_with("seen_") || field.starts_with("processed_")
            }
            _ => false,
        });
        if !ok {
            eprintln!(
                "warning: handler '{fname}' is named idempotent but does no \
                 dedupe check (call is_duplicate()/was_seen()/dedupe() or read \
                 from a 'seen_*'/'processed_*' field)"
            );
        }
    });
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
    fn program_without_handler_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn dedupe_fns_include_is_duplicate() {
        assert!(DEDUPE_FNS.contains(&"is_duplicate"));
        assert!(DEDUPE_FNS.contains(&"was_seen"));
    }
}
