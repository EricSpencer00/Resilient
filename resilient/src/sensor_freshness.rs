//! Ralph-Loop Uniqueness #2 — sensor-freshness enforcement.
//!
//! Robotics middleware (ROS, MAVLink, Cyphal) all surface the bug: code
//! reads a stale sensor and acts on it because the freshness check was
//! omitted. *No language* enforces freshness statically: ROS does it via
//! convention + message timestamps checked at runtime; SPARK doesn't model
//! sensor lifecycles; Rust gives you a `Result<T, Stale>` only if you wrap
//! it yourself.
//!
//! Resilient enforces, by type-name convention, that any function which
//! takes a `Sensor`/`Sensor<T>`/`&Sensor` parameter and reads its
//! `.value()` / `.read()` must, on every path leading to that read,
//! have ALREADY called `.is_fresh()` / `.fresh()` / `is_fresh(<param>)`.
//!
//! The ordering requirement matters: a freshness check that appears
//! AFTER the read is not a guard — it cannot prevent stale data from
//! being used. The analysis is CFG-aware: for sequential code, the
//! freshness call must precede the read; for if/else, the freshness
//! check must be present on both branches before any read in that branch.

#![allow(
    clippy::collapsible_if,
    clippy::collapsible_match,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::for_each_function;

const SENSOR_TYPE_PREFIXES: &[&str] = &["Sensor", "&Sensor", "&mut Sensor"];
const READ_METHODS: &[&str] = &["value", "read", "sample", "latest"];
const FRESH_METHODS: &[&str] = &["is_fresh", "fresh", "is_recent", "stamp"];
const FRESH_FREE_FNS: &[&str] = &["is_fresh", "assert_fresh"];

/// Ordering outcome for freshness-before-read analysis on a subtree.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FreshOrder {
    /// A read was found before any freshness check on this path.
    StaleRead,
    /// A freshness check was found; subsequent reads on this path are safe.
    FreshChecked,
    /// Neither a read nor a freshness check was found (neutral subtree).
    Neither,
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1218: fast-reject — skip for programs with no Sensor-typed parameters.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let has_sensor = stmts.iter().any(|s| {
        matches!(&s.node, Node::Function { parameters, .. }
            if parameters.iter().any(|(ty, _)| SENSOR_TYPE_PREFIXES.iter().any(|p| ty.starts_with(*p))))
    });
    if !has_sensor {
        return Ok(());
    }
    for_each_function(program, |name, params, body| {
        let sensors: Vec<&str> = params
            .iter()
            .filter(|(ty, _)| SENSOR_TYPE_PREFIXES.iter().any(|p| ty.starts_with(*p)))
            .map(|(_, n)| n.as_str())
            .collect();
        if sensors.is_empty() {
            return;
        }
        // Only warn for sensors that are actually read somewhere.
        let stale: Vec<&str> = sensors
            .iter()
            .copied()
            .filter(|s| reads_value(body, s))
            .filter(|s| {
                // A sensor read is unsafe when there's a path that reaches
                // the read without a prior freshness check.
                has_stale_read(body, s)
            })
            .collect();
        if !stale.is_empty() {
            eprintln!(
                "warning: function '{name}' reads sensor parameter(s) [{}] \
                 without a prior .is_fresh()/.fresh() check on all paths — \
                 may act on stale data (freshness check must precede the read)",
                stale.join(", ")
            );
        }
    });
    Ok(())
}

/// Returns `true` when there exists a path through `body` that reaches a read
/// of `sensor` without a prior freshness check.
fn has_stale_read(body: &Node, sensor: &str) -> bool {
    check_order(body, sensor) == FreshOrder::StaleRead
}

/// Returns the ordering outcome for `node`: whether a stale read, a fresh
/// check, or neither is found first on some execution path.
fn check_order(node: &Node, sensor: &str) -> FreshOrder {
    match node {
        Node::Block { stmts, .. } => check_stmts(stmts, sensor),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            let c = check_order(consequence, sensor);
            let a = match alternative {
                Some(alt) => check_order(alt, sensor),
                None => FreshOrder::Neither,
            };
            // If either branch has a stale read, the whole if is stale.
            if c == FreshOrder::StaleRead || a == FreshOrder::StaleRead {
                return FreshOrder::StaleRead;
            }
            // If both check freshness, the combined outcome is FreshChecked.
            if c == FreshOrder::FreshChecked && a == FreshOrder::FreshChecked {
                return FreshOrder::FreshChecked;
            }
            FreshOrder::Neither
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            check_order(body, sensor)
        }
        _ => {
            if is_fresh_check_node(node, sensor) {
                FreshOrder::FreshChecked
            } else if is_read_node(node, sensor) {
                FreshOrder::StaleRead
            } else {
                FreshOrder::Neither
            }
        }
    }
}

/// Scan a sequential list of statements, tracking state: once a freshness
/// check is seen, all subsequent reads are safe.
fn check_stmts(stmts: &[Node], sensor: &str) -> FreshOrder {
    let mut fresh_seen = false;
    for stmt in stmts {
        if fresh_seen {
            // Already checked — remaining reads are safe.
            continue;
        }
        if is_fresh_check_node(stmt, sensor) {
            fresh_seen = true;
            continue;
        }
        match stmt {
            Node::IfStatement {
                consequence,
                alternative,
                ..
            } => {
                let c = check_order(consequence, sensor);
                let a = match alternative {
                    Some(alt) => check_order(alt, sensor),
                    None => FreshOrder::Neither,
                };
                if c == FreshOrder::StaleRead || a == FreshOrder::StaleRead {
                    return FreshOrder::StaleRead;
                }
                if c == FreshOrder::FreshChecked && a == FreshOrder::FreshChecked {
                    fresh_seen = true;
                }
            }
            Node::Block { stmts: inner, .. } => {
                let result = check_stmts(inner, sensor);
                if result == FreshOrder::StaleRead {
                    return FreshOrder::StaleRead;
                }
                if result == FreshOrder::FreshChecked {
                    fresh_seen = true;
                }
            }
            _ => {
                if is_read_node(stmt, sensor) {
                    return FreshOrder::StaleRead;
                }
            }
        }
    }
    if fresh_seen {
        FreshOrder::FreshChecked
    } else {
        FreshOrder::Neither
    }
}

/// Returns `true` if `node` is a read call on `sensor`.
fn is_read_node(node: &Node, sensor: &str) -> bool {
    match node {
        Node::ExpressionStatement { expr, .. } => is_read_call(expr, sensor),
        Node::LetStatement { value, .. } => is_read_call(value, sensor),
        Node::ReturnStatement { value: Some(e), .. } => is_read_call(e, sensor),
        _ => is_read_call(node, sensor),
    }
}

fn is_read_call(node: &Node, sensor: &str) -> bool {
    if let Node::CallExpression {
        function,
        arguments,
        ..
    } = node
    {
        if let Node::FieldAccess { target, field, .. } = function.as_ref() {
            if READ_METHODS.contains(&field.as_str()) && is_param(target, sensor) {
                return true;
            }
        }
        if let Node::Identifier { name, .. } = function.as_ref() {
            if READ_METHODS.contains(&name.as_str())
                && arguments.iter().any(|a| is_param(a, sensor))
            {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if `node` is a freshness check for `sensor`.
fn is_fresh_check_node(node: &Node, sensor: &str) -> bool {
    match node {
        Node::ExpressionStatement { expr, .. } => is_fresh_call(expr, sensor),
        Node::LetStatement { value, .. } => is_fresh_call(value, sensor),
        Node::IfStatement { condition, .. } => is_fresh_call(condition, sensor),
        _ => is_fresh_call(node, sensor),
    }
}

fn is_fresh_call(node: &Node, sensor: &str) -> bool {
    if let Node::CallExpression {
        function,
        arguments,
        ..
    } = node
    {
        if let Node::FieldAccess { target, field, .. } = function.as_ref() {
            if FRESH_METHODS.contains(&field.as_str()) && is_param(target, sensor) {
                return true;
            }
        }
        if let Node::Identifier { name, .. } = function.as_ref() {
            if FRESH_FREE_FNS.contains(&name.as_str())
                && arguments.iter().any(|a| is_param(a, sensor))
            {
                return true;
            }
        }
    }
    false
}

fn reads_value(body: &Node, sensor: &str) -> bool {
    crate::uniqueness_walk::any_node(body, |n| is_read_call(n, sensor))
}

fn is_param(node: &Node, sensor: &str) -> bool {
    matches!(node, Node::Identifier { name, .. } if name == sensor)
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
    fn fresh_before_read_is_safe() {
        let src = r#"
fn read_temp(Sensor s) -> int {
    s.is_fresh();
    return s.value();
}
"#;
        let (prog, _) = parse(src);
        let has = any_stale_warning(&prog, "s");
        assert!(!has, "fresh check before read must not warn");
    }

    #[test]
    fn read_before_fresh_is_stale() {
        let src = r#"
fn read_temp(Sensor s) -> int {
    let v = s.value();
    s.is_fresh();
    return v;
}
"#;
        let (prog, _) = parse(src);
        let has = any_stale_warning(&prog, "s");
        assert!(has, "read before fresh check must be flagged as stale");
    }

    #[test]
    fn no_fresh_check_at_all_is_stale() {
        let src = r#"
fn read_temp(Sensor s) -> int {
    return s.value();
}
"#;
        let (prog, _) = parse(src);
        let has = any_stale_warning(&prog, "s");
        assert!(has, "read with no freshness check anywhere must be flagged");
    }

    #[test]
    fn if_else_both_fresh_before_read_is_safe() {
        let src = r#"
fn read_temp(Sensor s, bool condition) -> int {
    if condition {
        s.is_fresh();
        return s.value();
    } else {
        s.is_fresh();
        return s.value();
    }
}
"#;
        let (prog, _) = parse(src);
        let has = any_stale_warning(&prog, "s");
        assert!(!has, "both branches fresh before read must not warn");
    }

    #[test]
    fn if_else_one_branch_stale_is_detected() {
        let src = r#"
fn read_temp(Sensor s, bool ok) -> int {
    if ok {
        s.is_fresh();
        return s.value();
    } else {
        return s.value();
    }
}
"#;
        let (prog, _) = parse(src);
        let has = any_stale_warning(&prog, "s");
        assert!(
            has,
            "if-else where one branch reads without fresh check must warn"
        );
    }

    /// Helper: returns true if the function body for any sensor param has a stale read.
    fn any_stale_warning(program: &Node, sensor: &str) -> bool {
        let Node::Program(stmts) = program else {
            return false;
        };
        for s in stmts {
            if let Node::Function {
                body, parameters, ..
            } = &s.node
            {
                let is_sensor_param = parameters.iter().any(|(_, n)| n == sensor);
                if is_sensor_param && reads_value(body, sensor) && has_stale_read(body, sensor) {
                    return true;
                }
            }
        }
        false
    }
}
