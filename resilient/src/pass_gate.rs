//! RES-1585: shared marker pre-scan for the `<EXTENSION_PASSES>` fan-out.
//!
//! Most attribute-driven typechecker passes (`crash_only`,
//! `watchdog_feed`, `sensor_freshness`, `secret_erasure`,
//! `transaction_commit`, `reentrancy_guard`, `monotonic_field`,
//! `backpressure_safe`, `bounded_blocking`, `degraded_mode`,
//! `priority_inheritance`, `rate_limit_static`, …) open with a hand-coded
//! top-level scan to bail when their marker is absent. RES-1218,
//! RES-1222, RES-1224, RES-1228, RES-1232, RES-1252, RES-1254, RES-1262,
//! RES-1266, RES-1267, RES-1271, RES-1274, RES-1275 each landed one of
//! these fast-rejects independently — they walk `stmts.iter().any(...)`
//! per pass, so a program with 20 top-level functions does 18+ separate
//! top-level walks just to discover "no, this pass has nothing to do."
//!
//! This module collects the union of those markers in **one** walk and
//! exposes the membership API each pass needs. The typechecker computes
//! a `Markers` value once and uses it to gate the per-pass calls; the
//! pass's own fast-reject stays as defense in depth (and for callers
//! that don't go through `Markers`, e.g. the LSP server's
//! per-document re-check path).
//!
//! Scope: only top-level `Node::Function` statements, matching the
//! existing per-pass fast-rejects (which all destructure
//! `Node::Program(stmts)` and look at `stmt.node`). Functions nested
//! inside `ImplBlock` / `ModuleDecl` are deliberately not part of the
//! pre-scan — adding them would change behaviour vs the existing
//! passes' fast-reject, which the gate must match exactly to avoid
//! suppressing diagnostics.

use crate::Node;
use std::collections::HashSet;

/// Aggregated markers from a single top-level walk of the program.
#[derive(Debug, Default)]
pub(crate) struct Markers {
    /// Names of every top-level `Node::Function` in the program.
    pub fn_names: HashSet<String>,
    /// Distinct parameter types across every top-level
    /// `Node::Function`'s parameter list.
    pub param_types: HashSet<String>,
}

impl Markers {
    /// One top-level walk; collects every top-level fn name and the
    /// distinct set of parameter types its parameters reference.
    pub(crate) fn scan(program: &Node) -> Self {
        let mut m = Markers::default();
        if let Node::Program(stmts) = program {
            for stmt in stmts {
                if let Node::Function {
                    name, parameters, ..
                } = &stmt.node
                {
                    m.fn_names.insert(name.clone());
                    for (ty, _) in parameters {
                        m.param_types.insert(ty.clone());
                    }
                }
            }
        }
        m
    }

    /// True if any top-level fn name begins with one of `prefixes`.
    pub(crate) fn any_fn_name_with_prefix(&self, prefixes: &[&str]) -> bool {
        self.fn_names
            .iter()
            .any(|n| prefixes.iter().any(|p| n.starts_with(p)))
    }

    /// True if any top-level fn parameter type is an exact match for
    /// one of `types`.
    pub(crate) fn any_param_type_in(&self, types: &[&str]) -> bool {
        types.iter().any(|t| self.param_types.contains(*t))
    }

    /// True if any top-level fn parameter type begins with one of
    /// `prefixes`. Mirrors the `SENSOR_TYPE_PREFIXES` / `SECRET_TYPE_PREFIXES`
    /// style of marker that prefix-matches `&Foo` / `&mut Foo` variants
    /// in addition to the base type name.
    pub(crate) fn any_param_type_with_prefix(&self, prefixes: &[&str]) -> bool {
        self.param_types
            .iter()
            .any(|t| prefixes.iter().any(|p| t.starts_with(p)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span;

    fn function_stmt(name: &str, params: Vec<(&str, &str)>) -> span::Spanned<Node> {
        span::Spanned {
            node: Node::Function {
                name: name.to_string(),
                parameters: params
                    .into_iter()
                    .map(|(t, n)| (t.to_string(), n.to_string()))
                    .collect(),
                defaults: Vec::new(),
                body: Box::new(Node::Block {
                    stmts: Vec::new(),
                    span: span::Span::default(),
                }),
                requires: Vec::new(),
                ensures: Vec::new(),
                recovers_to: None,
                return_type: None,
                span: span::Span::default(),
                pure: false,
                effects: crate::EffectSet::io(),
                type_params: Vec::new(),
                type_param_bounds: Vec::new(),
                fails: Vec::new(),
            },
            span: span::Span::default(),
        }
    }

    #[test]
    fn scan_collects_fn_names() {
        let program = Node::Program(vec![
            function_stmt("foo", vec![]),
            function_stmt("crash_recover", vec![]),
            function_stmt("bar", vec![]),
        ]);
        let m = Markers::scan(&program);
        assert!(m.fn_names.contains("foo"));
        assert!(m.fn_names.contains("crash_recover"));
        assert!(m.fn_names.contains("bar"));
        assert_eq!(m.fn_names.len(), 3);
    }

    #[test]
    fn scan_collects_param_types() {
        let program = Node::Program(vec![
            function_stmt("a", vec![("int", "x"), ("Watchdog", "w")]),
            function_stmt("b", vec![("&Sensor", "s"), ("int", "y")]),
        ]);
        let m = Markers::scan(&program);
        assert!(m.param_types.contains("int"));
        assert!(m.param_types.contains("Watchdog"));
        assert!(m.param_types.contains("&Sensor"));
    }

    #[test]
    fn any_fn_name_with_prefix_matches() {
        let program = Node::Program(vec![
            function_stmt("crash_main", vec![]),
            function_stmt("regular", vec![]),
        ]);
        let m = Markers::scan(&program);
        assert!(m.any_fn_name_with_prefix(&["crash_"]));
        assert!(!m.any_fn_name_with_prefix(&["xyz_"]));
        assert!(m.any_fn_name_with_prefix(&["xyz_", "crash_"]));
    }

    #[test]
    fn any_param_type_in_exact_match() {
        let program = Node::Program(vec![function_stmt("h", vec![("Watchdog", "w")])]);
        let m = Markers::scan(&program);
        assert!(m.any_param_type_in(&["Watchdog"]));
        assert!(!m.any_param_type_in(&["Sensor"]));
        assert!(m.any_param_type_in(&["int", "Watchdog"]));
    }

    #[test]
    fn any_param_type_with_prefix_matches_ref_forms() {
        let program = Node::Program(vec![function_stmt("h", vec![("&mut Sensor", "s")])]);
        let m = Markers::scan(&program);
        assert!(m.any_param_type_with_prefix(&["Sensor", "&Sensor", "&mut Sensor"]));
        assert!(!m.any_param_type_with_prefix(&["Watchdog"]));
    }

    #[test]
    fn scan_on_non_program_node_is_empty() {
        let m = Markers::scan(&Node::Block {
            stmts: Vec::new(),
            span: span::Span::default(),
        });
        assert!(m.fn_names.is_empty());
        assert!(m.param_types.is_empty());
    }

    #[test]
    fn scan_on_empty_program_is_empty() {
        let m = Markers::scan(&Node::Program(Vec::new()));
        assert!(m.fn_names.is_empty());
        assert!(m.param_types.is_empty());
    }
}
