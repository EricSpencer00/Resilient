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
//! `.value()` / `.read()` must, somewhere on a path leading to that read,
//! have called `.is_fresh()` / `.fresh()` / `is_fresh(<param>)` on the
//! same parameter. Otherwise we warn that the read may consume stale data.

#![allow(
    clippy::collapsible_if,
    clippy::doc_lazy_continuation,
    clippy::single_match
)]

use crate::Node;
use crate::uniqueness_walk::{any_node, for_each_function};

const SENSOR_TYPE_PREFIXES: &[&str] = &["Sensor", "&Sensor", "&mut Sensor"];
const READ_METHODS: &[&str] = &["value", "read", "sample", "latest"];
const FRESH_METHODS: &[&str] = &["is_fresh", "fresh", "is_recent", "stamp"];
const FRESH_FREE_FNS: &[&str] = &["is_fresh", "assert_fresh"];

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1218: fast-reject — see watchdog_feed for the same pattern.
    // Skip the closure dispatch + per-fn allocation for programs
    // that declare no `Sensor*`-typed parameter.
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
        let reads = sensors
            .iter()
            .filter(|s| reads_value(body, s))
            .copied()
            .collect::<Vec<_>>();
        if reads.is_empty() {
            return;
        }
        let unchecked: Vec<&str> = reads
            .into_iter()
            .filter(|s| !checks_fresh(body, s))
            .collect();
        if !unchecked.is_empty() {
            eprintln!(
                "warning: function '{name}' reads sensor parameter(s) [{}] without \
                 first checking .is_fresh()/.fresh()/.is_recent() — may act on stale data",
                unchecked.join(", ")
            );
        }
    });
    Ok(())
}

fn reads_value(body: &Node, sensor: &str) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
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
            false
        }
        _ => false,
    })
}

fn checks_fresh(body: &Node, sensor: &str) -> bool {
    any_node(body, |n| match n {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
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
            false
        }
        _ => false,
    })
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
}
