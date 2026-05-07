//! Ralph-Loop Uniqueness #1 — watchdog-feed enforcement.
//!
//! Hardware watchdogs are the canonical liveness primitive on safety-critical
//! systems: if the firmware doesn't periodically "feed" the watchdog, it
//! resets the device. *No mainstream language* enforces, at compile time,
//! that a function holding a watchdog actually feeds it. C, Rust, Ada, and
//! SPARK all leave it to documentation and reviewer eyeballs.
//!
//! Resilient enforces it as a real static check:
//!
//!   - Any function with a parameter whose declared type is `Watchdog`
//!     (or `&Watchdog`, `&mut Watchdog`) must contain at least one
//!     reachable call equivalent to feeding that watchdog. We accept:
//!       * `<param>.feed()` / `<param>.kick()` / `<param>.pet()` /
//!         `<param>.reset()` method calls, OR
//!       * a free function call `feed_watchdog(<param>)` /
//!         `kick_watchdog(<param>)`,
//!     anywhere in the body.
//!   - A function whose body has zero such calls emits a warning that
//!     points at the function name.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function};

const WATCHDOG_TYPES: &[&str] = &["Watchdog", "&Watchdog", "&mut Watchdog"];
const FEED_METHODS: &[&str] = &["feed", "kick", "pet", "reset"];
const FEED_FREE_FNS: &[&str] = &["feed_watchdog", "kick_watchdog", "pet_watchdog"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    for_each_function(program, |name, params, body| {
        let watchdogs: Vec<&str> = params
            .iter()
            .filter(|(ty, _)| WATCHDOG_TYPES.contains(&ty.as_str()))
            .map(|(_, n)| n.as_str())
            .collect();
        if watchdogs.is_empty() {
            return;
        }
        if !body_feeds_any(body, &watchdogs) {
            eprintln!(
                "warning: function '{name}' takes Watchdog parameter(s) [{}] but \
                 never calls .feed()/.kick()/.pet()/.reset() or feed_watchdog() — \
                 the watchdog will starve and reset the device",
                watchdogs.join(", ")
            );
        }
    });
    Ok(())
}

fn body_feeds_any(body: &Node, params: &[&str]) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::FieldAccess { target, field, .. } = function.as_ref() {
                if FEED_METHODS.contains(&field.as_str()) && is_param(target, params) {
                    return true;
                }
            }
            if let Node::Identifier { name, .. } = function.as_ref() {
                if FEED_FREE_FNS.contains(&name.as_str())
                    && arguments.iter().any(|a| is_param(a, params))
                {
                    return true;
                }
            }
            false
        }
        _ => false,
    })
}

fn is_param(node: &Node, params: &[&str]) -> bool {
    matches!(node, Node::Identifier { name, .. } if params.contains(&name.as_str()))
}
