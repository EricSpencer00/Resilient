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
    clippy::single_match
)]

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
    // RES-2002: match the Actor variant directly and walk handlers in a
    // single pass. The previous helper-pair (`collect_handler_names` +
    // `actor_name_of`) cloned the actor name plus every handler name
    // into a `Vec<String>`, then iterated that Vec twice. All of that
    // owned-data extraction was wasted — the Actor variant outlives
    // every use here.
    let Node::Actor {
        name: actor_name,
        handlers,
        ..
    } = node
    else {
        return;
    };
    let mut has_stop = false;
    let mut has_drain = false;
    for h in handlers {
        let n = h.name.as_str();
        if STOP_HANDLERS.contains(&n) {
            has_stop = true;
        }
        if DRAIN_HANDLERS.contains(&n) {
            has_drain = true;
        }
    }
    if has_stop && !has_drain {
        eprintln!(
            "warning: actor '{actor_name}' declares a shutdown/terminate handler but \
             no drain/flush handler — un-processed messages may be discarded on exit"
        );
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
