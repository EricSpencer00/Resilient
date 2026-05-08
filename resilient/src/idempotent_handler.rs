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
