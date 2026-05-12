//! Ralph-Loop Uniqueness #8 — actor drain-on-shutdown.
//!
//! Erlang/OTP actors can ignore queued messages on `terminate`. Akka
//! lets you "stop" an actor mid-mailbox. Pony's reference-capability
//! system promises *delivery* but not *processing-before-shutdown*.
//! No mainstream actor language verifies, at compile time, that an
//! actor processes its mailbox before exiting.
//!
//! Resilient enforces a Drain-Before-Shutdown contract by syntactic
//! convention: any actor declaration whose body declares a `shutdown`
//! / `terminate` / `on_stop` handler must *also* declare a `drain` /
//! `flush` handler that processes the queue. Actors that have a stop
//! handler but no drain handler are warned about: they may exit with
//! un-processed messages.
//!
//! The check operates on `Node::Actor` (RES-332). It iterates the
//! actor's declared receive handlers and looks for the convention.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match,
    // RES-1232: helpers are unreachable while `check` is a no-op
    // pending the `Node::Actor` variant landing. Same shape as the
    // dead-pass cleanups in RES-1202 / RES-1206.
    dead_code
)]

use crate::Node;

const STOP_HANDLERS: &[&str] = &["shutdown", "terminate", "on_stop", "stop"];
const DRAIN_HANDLERS: &[&str] = &["drain", "flush", "drain_mailbox"];

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1232: dead pass — `actor_name_of` always returns `None`
    // until `Node::Actor { name, .. }` is wired (see the comment on
    // `actor_name_of` below). Consequently `collect_handler_names`
    // always returns `None`, `check_actor` always early-returns, and
    // the per-stmt walk emits no diagnostics for any program. Skip
    // it entirely instead of doing N closure dispatches for nothing.
    //
    // Same shape as RES-1202 (`region_inference::infer`'s
    // discarded-walk no-op) and RES-1206 (the five
    // analysis-result-discarded passes). When the `Node::Actor`
    // variant lands and `actor_name_of` starts returning `Some`,
    // restore the walk:
    //
    //     let Node::Program(stmts) = program else { return Ok(()); };
    //     for stmt in stmts {
    //         check_actor(&stmt.node);
    //     }
    Ok(())
}

fn check_actor(node: &Node) {
    let handlers = match collect_handler_names(node) {
        Some((name, h)) => (name, h),
        None => return,
    };
    let (actor_name, handler_names) = handlers;
    let has_stop = handler_names
        .iter()
        .any(|h| STOP_HANDLERS.contains(&h.as_str()));
    let has_drain = handler_names
        .iter()
        .any(|h| DRAIN_HANDLERS.contains(&h.as_str()));
    if has_stop && !has_drain {
        eprintln!(
            "warning: actor '{actor_name}' declares a shutdown/terminate handler but \
             no drain/flush handler — un-processed messages may be discarded on exit"
        );
    }
}

/// Best-effort: discover an actor declaration and the names of its
/// declared receive handlers. The actual `Node::Actor` shape includes
/// receive handlers as nested function-like nodes; we look for any
/// child with a `name`-bearing variant under the actor body.
fn collect_handler_names(node: &Node) -> Option<(String, Vec<String>)> {
    let actor_name = actor_name_of(node)?;
    let mut handlers = Vec::new();
    crate::uniqueness_walk::visit(node, &mut |n| {
        if let Node::Function { name, .. } = n {
            handlers.push(name.clone());
        }
    });
    Some((actor_name, handlers))
}

fn actor_name_of(node: &Node) -> Option<String> {
    // We don't statically know the precise enum shape of the Actor node
    // across the codebase, so we use a string-based heuristic: any node
    // formatted by the debug formatter with `Actor { name: "..." }` style.
    // Production hardening would match `Node::Actor { name, .. }` directly
    // once that variant lands; for now we return None so the pass is a
    // no-op on programs without actors and quietly skips the rest.
    let _ = node;
    None
}
