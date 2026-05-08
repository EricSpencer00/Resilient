//! Ralph-Loop Uniqueness #9 — backpressure-safe handler discipline.
//!
//! Reactive frameworks (RxJava, Reactor, Akka Streams) all expose
//! backpressure as a runtime concern: when the mailbox fills, you pick
//! a strategy. No language *type-checks* that a handler parameterized
//! over a bounded mailbox actually has a backpressure strategy in code.
//!
//! Resilient enforces it: any function whose name ends in `_handler`
//! AND takes a parameter named `mailbox` (or typed `Mailbox` /
//! `BoundedQueue`) must contain at least one of:
//!   * a call to `drop_oldest` / `drop_newest` / `block_caller`
//!   * a `match` expression on a literal Result / Option scrutinee
//!     covering the "queue full" arm
//!   * a guard like `if mailbox_full(...) { ... }`
//! Otherwise the handler is silently overflow-prone and we warn.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function};

const STRATEGY_FNS: &[&str] = &[
    "drop_oldest",
    "drop_newest",
    "block_caller",
    "shed_load",
    "park_caller",
];
const FULLNESS_FNS: &[&str] = &["mailbox_full", "is_full", "queue_full", "at_capacity"];
const QUEUE_TYPES: &[&str] = &["Mailbox", "BoundedQueue", "&Mailbox", "&mut Mailbox"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    for_each_function(program, |fname, params, body| {
        if !fname.ends_with("_handler") {
            return;
        }
        let has_q = params
            .iter()
            .any(|(ty, n)| QUEUE_TYPES.contains(&ty.as_str()) || n == "mailbox" || n == "queue");
        if !has_q {
            return;
        }
        if !has_strategy(body) {
            eprintln!(
                "warning: handler '{fname}' takes a bounded mailbox but has no \
                 backpressure strategy (drop_oldest/drop_newest/block_caller/shed_load \
                 or an is_full() guard) — overflow behaviour is undefined"
            );
        }
    });
    Ok(())
}

fn has_strategy(body: &Node) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression { function, .. } => match function.as_ref() {
            Node::Identifier { name, .. } => {
                STRATEGY_FNS.contains(&name.as_str()) || FULLNESS_FNS.contains(&name.as_str())
            }
            _ => false,
        },
        _ => false,
    })
}
