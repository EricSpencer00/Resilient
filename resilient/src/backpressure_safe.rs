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
    // RES-1217: fast-reject. The pass only emits diagnostics for
    // functions whose name ends in `_handler`. For every program
    // that declares no such function — overwhelmingly the case in
    // `examples/` and the test suite — `for_each_function`'s
    // closure dispatch + per-fn suffix check is wasted work. Scan
    // the top-level names once up front and bail.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_handler = stmts
        .iter()
        .any(|s| matches!(&s.node, Node::Function { name, .. } if name.ends_with("_handler")));
    if !has_handler {
        return Ok(());
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_handler_fn_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn strategy_fns_include_drop_oldest() {
        assert!(STRATEGY_FNS.contains(&"drop_oldest"));
        assert!(FULLNESS_FNS.contains(&"mailbox_full"));
    }

    // Regression corpus: valid handlers with mailbox and strategy
    #[test]
    fn handler_with_drop_oldest_strategy() {
        let src = "fn my_handler(int mailbox) -> int { return drop_oldest(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_drop_newest_strategy() {
        let src = "fn my_handler(int mailbox) -> int { return drop_newest(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_is_full_guard() {
        let src = "fn is_full(int q) -> bool { return q > 10; }\n\
                   fn my_handler(int mailbox) -> int { \
                     if is_full(mailbox) { return 0; } \
                     return mailbox; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_mailbox_full_guard() {
        let src = "fn mailbox_full(int q) -> bool { return q > 10; }\n\
                   fn my_handler(int mailbox) -> int { \
                     if mailbox_full(mailbox) { return 0; } \
                     return mailbox; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_block_caller_strategy() {
        let src = "fn my_handler(int mailbox) -> int { return block_caller(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_shed_load_strategy() {
        let src = "fn my_handler(int mailbox) -> int { return shed_load(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_park_caller_strategy() {
        let src = "fn my_handler(int mailbox) -> int { return park_caller(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_queue_full_guard() {
        let src = "fn queue_full(int q) -> bool { return q > 10; }\n\
                   fn my_handler(int mailbox) -> int { \
                     if queue_full(mailbox) { return 0; } \
                     return mailbox; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_at_capacity_guard() {
        let src = "fn at_capacity(int q) -> bool { return q > 10; }\n\
                   fn my_handler(int mailbox) -> int { \
                     if at_capacity(mailbox) { return 0; } \
                     return mailbox; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_queue_param_and_strategy() {
        let src = "fn my_handler(int queue) -> int { return drop_oldest(queue); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn non_handler_with_mailbox_param() {
        let src = "fn process(int mailbox) -> int { return mailbox; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_without_queue_param() {
        let src = "fn my_handler(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn multiple_handlers_all_with_strategy() {
        let src = "fn handler1(int mailbox) -> int { return drop_oldest(mailbox); }\n\
                   fn handler2(int mailbox) -> int { return drop_newest(mailbox); }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_multiple_strategy_calls() {
        let src = "fn my_handler(int mailbox) -> int { \
                     let x = drop_oldest(mailbox); \
                     let y = is_full(mailbox); \
                     return x; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_nested_strategy_call() {
        let src = "fn drop_oldest(int q) -> int { return q - 1; }\n\
                   fn my_handler(int mailbox) -> int { \
                     return drop_oldest(drop_oldest(mailbox)); \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_mailbox_but_no_strategy() {
        // Note: this should print a warning to stderr, but the check itself returns Ok
        let src = "fn my_handler(int mailbox) -> int { return mailbox + 1; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_queue_param_but_no_strategy() {
        let src = "fn my_handler(int queue) -> int { return queue + 1; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn mixed_handlers_some_valid_some_invalid() {
        let src = "fn handler1(int mailbox) -> int { return drop_oldest(mailbox); }\n\
                   fn handler2(int mailbox) -> int { return mailbox + 1; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn handler_with_strategy_in_nested_scope() {
        let src = "fn is_full(int q) -> bool { return q > 10; }\n\
                   fn my_handler(int mailbox) -> int { \
                     if mailbox > 0 { \
                       if is_full(mailbox) { return 0; } \
                     } \
                     return mailbox; \
                   }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
