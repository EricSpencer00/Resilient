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

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, clippy::single_match)]

use crate::Node;

const STOP_HANDLERS: &[&str] = &["shutdown", "terminate", "on_stop", "stop"];
const DRAIN_HANDLERS: &[&str] = &["drain", "flush", "drain_mailbox"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1232: `Node::Actor { name, handlers, .. }` is wired; activate the walk.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for stmt in stmts {
        check_actor(&stmt.node);
    }
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
    if let Node::Actor { handlers, .. } = node {
        let names = handlers.iter().map(|h| h.name.clone()).collect();
        Some((actor_name, names))
    } else {
        None
    }
}

fn actor_name_of(node: &Node) -> Option<String> {
    if let Node::Actor { name, .. } = node {
        Some(name.clone())
    } else {
        None
    }
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
    fn program_without_actor_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn stop_drain_handlers_detected() {
        assert!(STOP_HANDLERS.contains(&"shutdown"));
        assert!(DRAIN_HANDLERS.contains(&"drain"));
    }
}
