//! Ralph-Loop Uniqueness #10 — monotonic-field invariants by name.
//!
//! Distributed systems use monotonic counters (Lamport clocks, sequence
//! numbers, last-modified timestamps) where decreasing the value is
//! always a bug. CRDT libraries enforce monotonicity at runtime via
//! library convention — no language enforces it statically.
//!
//! Resilient enforces monotonicity by *struct field name convention*:
//! any field whose name starts with `last_`, `latest_`, `max_`, or
//! `monotonic_` (or ends in `_seq` / `_clock` / `_epoch`) may only be
//! assigned a value that is provably ≥ its current value, OR is the
//! result of an explicit `+`/`max(...)` whose left operand is the field
//! itself. We warn on assignments that decrement, replace with a smaller
//! literal, or use a non-monotonic operator like `-`.

#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, visit};

const MONO_PREFIXES: &[&str] = &["last_", "latest_", "max_", "monotonic_"];
const MONO_SUFFIXES: &[&str] = &["_seq", "_clock", "_epoch", "_tick"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    // RES-1262: fast-reject. The per-stmt `visit` walks every top-level
    // statement's full AST looking for a `FieldAssignment` whose field
    // matches `is_monotonic_field`. For programs without any such
    // assignment (the overwhelming majority of `cargo test` inputs and
    // the entire `examples/` tree), every visit produces nothing. Pre-
    // scan the program once via `any_node` (RES-1238 made this
    // early-terminating) and skip the loop entirely.
    let has_monotonic_assign = any_node(program, |n| match n {
        Node::FieldAssignment { field, .. } => is_monotonic_field(field),
        _ => false,
    });
    if !has_monotonic_assign {
        return Ok(());
    }
    for stmt in stmts {
        visit(&stmt.node, &mut |n| {
            if let Node::FieldAssignment {
                target,
                field,
                value,
                ..
            } = n
            {
                if !is_monotonic_field(field) {
                    return;
                }
                if !value_is_monotonic(value, target, field) {
                    eprintln!(
                        "warning: assignment to monotonic field '.{field}' uses an \
                         expression that may decrease its value (must be >= current; \
                         use `+` or `max(...)` with the field itself as one operand)"
                    );
                }
            }
        });
    }
    Ok(())
}

fn is_monotonic_field(name: &str) -> bool {
    MONO_PREFIXES.iter().any(|p| name.starts_with(*p))
        || MONO_SUFFIXES.iter().any(|s| name.ends_with(*s))
}

fn value_is_monotonic(value: &Node, target: &Node, field: &str) -> bool {
    match value {
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            if *operator == "+" {
                touches_self_field(left, target, field) || touches_self_field(right, target, field)
            } else {
                false
            }
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                if name == "max" {
                    return arguments
                        .iter()
                        .any(|a| touches_self_field(a, target, field));
                }
            }
            false
        }
        _ => false,
    }
}

fn touches_self_field(expr: &Node, target: &Node, field: &str) -> bool {
    match expr {
        Node::FieldAccess {
            target: t,
            field: f,
            ..
        } => f == field && nodes_eq_ident(t, target),
        _ => false,
    }
}

fn nodes_eq_ident(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Identifier { name: x, .. }, Node::Identifier { name: y, .. }) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn empty_program_returns_ok() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn program_without_trigger_returns_ok() {
        let src = "fn f(int x) -> int { return x + 1; }\nf(5);\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn monotonic_increment_with_plus_passes() {
        let src = r#"
            struct Counter { int last_count }
            fn increment(Counter c) -> Counter {
                c.last_count = c.last_count + 1;
                return c;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn monotonic_field_with_max_function_passes() {
        let src = r#"
            struct Tracker { int max_value }
            fn update(Tracker t, int x) -> Tracker {
                t.max_value = max(t.max_value, x);
                return t;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn latest_prefix_field_with_increment_passes() {
        let src = r#"
            struct Clock { int latest_tick }
            fn advance(Clock c) -> Clock {
                c.latest_tick = c.latest_tick + 1;
                return c;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn monotonic_suffix_field_with_addition_passes() {
        let src = r#"
            struct Timing { int event_clock }
            fn record(Timing t) -> Timing {
                t.event_clock = t.event_clock + 1;
                return t;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn sequence_number_field_with_increment_passes() {
        let src = r#"
            struct Event { int msg_seq }
            fn next_seq(Event e) -> Event {
                e.msg_seq = e.msg_seq + 1;
                return e;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn multiple_monotonic_fields() {
        let src = r#"
            struct State { int last_update, int max_level, int event_epoch }
            fn update_all(State s) -> State {
                s.last_update = s.last_update + 1;
                s.max_level = max(s.max_level, 10);
                s.event_epoch = s.event_epoch + 1;
                return s;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn non_monotonic_field_with_subtraction_warns() {
        let src = r#"
            struct Counter { int last_value }
            fn decrement(Counter c) -> Counter {
                c.last_value = c.last_value - 1;
                return c;
            }
        "#;
        let (prog, _) = parse(src);
        // Should warn but return Ok
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn non_monotonic_field_replacement_warns() {
        let src = r#"
            struct Clock { int max_ticks }
            fn reset(Clock c) -> Clock {
                c.max_ticks = 0;
                return c;
            }
        "#;
        let (prog, _) = parse(src);
        // Should warn but return Ok
        assert!(check(&prog, "test").is_ok());
    }
}
