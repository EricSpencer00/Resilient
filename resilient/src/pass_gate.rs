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
//! Scope: walks the program AST via `uniqueness_walk::visit`, the
//! same recursion the per-pass `any_node` calls use. That descends
//! into `Function` bodies, `Block` statements, `LetStatement`
//! values, `IfStatement` arms, `CallExpression` arguments,
//! `FieldAccess` / `FieldAssignment` targets, etc. — but stops at
//! `ImplBlock` / `ModuleDecl` (the same boundary as the existing
//! `walk_children` impl). Functions nested in those constructs are
//! not visible to the existing per-pass fast-rejects either, so the
//! gate-vs-pass equivalence holds.

use crate::Node;
use std::collections::HashSet;

/// Aggregated markers from a single whole-AST walk of the program.
///
/// Top-level fields (`fn_names`, `param_types`) back the RES-1585 /
/// RES-1590 gates on attribute-driven Ralph-Loop passes. Whole-AST
/// fields (`param_names`, `let_names`, `field_names_*`,
/// `call_idents`) back the RES-1593 gates on deep-scan passes that
/// previously each ran their own early-terminating `any_node` walk.
#[derive(Debug, Default)]
pub(crate) struct Markers {
    /// Names of every `Node::Function` in the program (top-level and
    /// nested via the standard `uniqueness_walk::visit` descent).
    pub fn_names: HashSet<String>,
    /// Distinct parameter types across every `Node::Function`'s
    /// parameter list.
    pub param_types: HashSet<String>,
    /// Distinct parameter *names* across every `Node::Function`'s
    /// parameter list. Used by the `numeric_units` gate, which seeds
    /// its units map from both let bindings and fn parameters.
    pub param_names: HashSet<String>,
    /// Names of every `Node::LetStatement` binding anywhere in the
    /// program. Used by the `saturation_required` and `numeric_units`
    /// gates.
    pub let_names: HashSet<String>,
    /// Field names appearing on the left of `Node::FieldAssignment`
    /// (`x.f = …`). Used by the `audit_log_required` gate.
    pub field_names_assigned: HashSet<String>,
    /// Field names appearing on `Node::FieldAccess` (`x.f`). Used by
    /// the `age_bounded_data` gate.
    pub field_names_accessed: HashSet<String>,
    /// Identifier names that appear as the *function* of a
    /// `Node::CallExpression`. Used by the `epoch_ordering` and
    /// `toctou_guard` gates.
    pub call_idents: HashSet<String>,
}

impl Markers {
    /// One whole-AST walk via `uniqueness_walk::visit`. Collects
    /// every marker source the gates below consult. Cost: O(N) for
    /// an N-node AST, paid once per type-check; saves up to six
    /// early-terminating `any_node` walks in the deep-scan passes
    /// below (RES-1593) plus the top-level walks (RES-1585 / 1590).
    pub(crate) fn scan(program: &Node) -> Self {
        let mut m = Markers::default();
        crate::uniqueness_walk::visit(program, &mut |n| match n {
            Node::Function {
                name, parameters, ..
            } => {
                m.fn_names.insert(name.clone());
                for (ty, pname) in parameters {
                    m.param_types.insert(ty.clone());
                    m.param_names.insert(pname.clone());
                }
            }
            Node::LetStatement { name, .. } => {
                m.let_names.insert(name.clone());
            }
            Node::FieldAssignment { field, .. } => {
                m.field_names_assigned.insert(field.clone());
            }
            Node::FieldAccess { field, .. } => {
                m.field_names_accessed.insert(field.clone());
            }
            Node::CallExpression { function, .. } => {
                if let Node::Identifier { name, .. } = function.as_ref() {
                    m.call_idents.insert(name.clone());
                }
            }
            _ => {}
        });
        m
    }

    /// True if any top-level fn name begins with one of `prefixes`.
    pub(crate) fn any_fn_name_with_prefix(&self, prefixes: &[&str]) -> bool {
        self.fn_names
            .iter()
            .any(|n| prefixes.iter().any(|p| n.starts_with(p)))
    }

    /// True if any top-level fn name ends with one of `suffixes`.
    ///
    /// RES-1590: backs the gate for `bounded_blocking`,
    /// `idempotent_handler`, `rate_limit_static`, `stack_budget`,
    /// `heap_budget`, and `bandwidth_budget` — all of which look for
    /// suffix-tagged fn names (`_bound{N}`, `_idempotent`,
    /// `_oncepertick`, `_stack{N}`, `_alloc{N}`, `_iobytes{N}`) as
    /// their entry-point marker.
    pub(crate) fn any_fn_name_with_suffix(&self, suffixes: &[&str]) -> bool {
        self.fn_names
            .iter()
            .any(|n| suffixes.iter().any(|s| n.ends_with(s)))
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

    /// True if any `Node::LetStatement` binding name ends with one
    /// of `suffixes`. Backs the `saturation_required` and
    /// `numeric_units` gates.
    pub(crate) fn any_let_name_with_suffix(&self, suffixes: &[&str]) -> bool {
        self.let_names
            .iter()
            .any(|n| suffixes.iter().any(|s| n.ends_with(s)))
    }

    /// True if any fn parameter name ends with one of `suffixes`.
    /// Used together with `any_let_name_with_suffix` by the
    /// `numeric_units` gate, which seeds units from both sources.
    pub(crate) fn any_param_name_with_suffix(&self, suffixes: &[&str]) -> bool {
        self.param_names
            .iter()
            .any(|n| suffixes.iter().any(|s| n.ends_with(s)))
    }

    /// True if any field name appearing on the left of a
    /// `Node::FieldAssignment` starts with one of `prefixes` or ends
    /// with one of `suffixes`. The `audit_log_required` gate fires
    /// on `audited_*` field prefix OR `*_audited` field suffix.
    pub(crate) fn any_field_assigned_with_prefix_or_suffix(
        &self,
        prefixes: &[&str],
        suffixes: &[&str],
    ) -> bool {
        self.field_names_assigned.iter().any(|f| {
            prefixes.iter().any(|p| f.starts_with(p)) || suffixes.iter().any(|s| f.ends_with(s))
        })
    }

    /// True if any field name appearing on `Node::FieldAccess` ends
    /// with one of `suffixes`. Backs the `age_bounded_data` gate
    /// (looks for `*_at` fields).
    pub(crate) fn any_field_accessed_with_suffix(&self, suffixes: &[&str]) -> bool {
        self.field_names_accessed
            .iter()
            .any(|f| suffixes.iter().any(|s| f.ends_with(s)))
    }

    /// True if any `Node::CallExpression` whose function is an
    /// `Identifier` has a name ending with one of `suffixes`. Backs
    /// the `toctou_guard` gate (`*_exists` / `*_is_valid` / …).
    pub(crate) fn any_call_ident_with_suffix(&self, suffixes: &[&str]) -> bool {
        self.call_idents
            .iter()
            .any(|n| suffixes.iter().any(|s| n.ends_with(s)))
    }

    /// True if any `Node::CallExpression` whose function is an
    /// `Identifier` has a name containing one of `substrs`. Backs
    /// the `epoch_ordering` gate, which matches names of shape
    /// `*_epoch<N>` via `rfind("_epoch")` rather than a fixed suffix.
    pub(crate) fn any_call_ident_containing(&self, substrs: &[&str]) -> bool {
        self.call_idents
            .iter()
            .any(|n| substrs.iter().any(|s| n.contains(s)))
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
    fn any_fn_name_with_suffix_matches() {
        let program = Node::Program(vec![
            function_stmt("read_buffer_bound2", vec![]),
            function_stmt("process_idempotent", vec![]),
            function_stmt("plain", vec![]),
        ]);
        let m = Markers::scan(&program);
        assert!(m.any_fn_name_with_suffix(&["_bound2"]));
        assert!(m.any_fn_name_with_suffix(&["_idempotent"]));
        assert!(!m.any_fn_name_with_suffix(&["_zzz"]));
        // Any matching suffix wins.
        assert!(m.any_fn_name_with_suffix(&["_zzz", "_bound2", "_other"]));
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

    // RES-1593: helpers for deep-scan markers. The constructed AST
    // fragments below intentionally use only the fields each gate
    // exercises (the rest stay default) — building a fully-populated
    // `Node::Function` for every test would be noise.

    fn id(name: &str) -> Box<Node> {
        Box::new(Node::Identifier {
            name: name.to_string(),
            span: span::Span::default(),
        })
    }

    fn let_stmt(name: &str) -> Node {
        Node::LetStatement {
            name: name.to_string(),
            value: Box::new(Node::IntegerLiteral {
                value: 0,
                span: span::Span::default(),
            }),
            type_annot: None,
            span: span::Span::default(),
        }
    }

    fn field_access(target: &str, field: &str) -> Node {
        Node::FieldAccess {
            target: id(target),
            field: field.to_string(),
            span: span::Span::default(),
        }
    }

    fn field_assign(target: &str, field: &str) -> Node {
        Node::FieldAssignment {
            target: id(target),
            field: field.to_string(),
            value: Box::new(Node::IntegerLiteral {
                value: 1,
                span: span::Span::default(),
            }),
            span: span::Span::default(),
        }
    }

    fn call(fn_name: &str) -> Node {
        Node::CallExpression {
            function: id(fn_name),
            arguments: Vec::new(),
            span: span::Span::default(),
        }
    }

    fn fn_with_body(name: &str, params: Vec<(&str, &str)>, body: Vec<Node>) -> span::Spanned<Node> {
        span::Spanned {
            node: Node::Function {
                name: name.to_string(),
                parameters: params
                    .into_iter()
                    .map(|(t, n)| (t.to_string(), n.to_string()))
                    .collect(),
                defaults: Vec::new(),
                body: Box::new(Node::Block {
                    stmts: body,
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
    fn scan_collects_let_names_and_param_names() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![("int", "duration_ms"), ("int", "n")],
            vec![let_stmt("brightness_pwm"), let_stmt("plain")],
        )]);
        let m = Markers::scan(&program);
        assert!(m.let_names.contains("brightness_pwm"));
        assert!(m.let_names.contains("plain"));
        assert!(m.param_names.contains("duration_ms"));
        assert!(m.param_names.contains("n"));
    }

    #[test]
    fn scan_collects_field_assignments_and_accesses() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![
                field_assign("self", "audited_balance"),
                Node::ExpressionStatement {
                    expr: Box::new(field_access("self", "updated_at")),
                    span: span::Span::default(),
                },
            ],
        )]);
        let m = Markers::scan(&program);
        assert!(m.field_names_assigned.contains("audited_balance"));
        assert!(m.field_names_accessed.contains("updated_at"));
    }

    #[test]
    fn scan_collects_call_idents() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![
                Node::ExpressionStatement {
                    expr: Box::new(call("file_exists")),
                    span: span::Span::default(),
                },
                Node::ExpressionStatement {
                    expr: Box::new(call("read_epoch3")),
                    span: span::Span::default(),
                },
            ],
        )]);
        let m = Markers::scan(&program);
        assert!(m.call_idents.contains("file_exists"));
        assert!(m.call_idents.contains("read_epoch3"));
    }

    #[test]
    fn any_let_name_with_suffix_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![let_stmt("led_pwm"), let_stmt("plain")],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_let_name_with_suffix(&["_pwm", "_duty"]));
        assert!(!m.any_let_name_with_suffix(&["_zzz"]));
    }

    #[test]
    fn any_param_name_with_suffix_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![("int", "delay_ms"), ("int", "n")],
            vec![],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_param_name_with_suffix(&["_ms"]));
        assert!(!m.any_param_name_with_suffix(&["_kg"]));
    }

    #[test]
    fn any_field_assigned_with_prefix_or_suffix_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![field_assign("self", "audited_balance")],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_field_assigned_with_prefix_or_suffix(&["audited_"], &[]));
        assert!(!m.any_field_assigned_with_prefix_or_suffix(&[], &["_zzz"]));

        let program2 = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![field_assign("self", "balance_audited")],
        )]);
        let m2 = Markers::scan(&program2);
        assert!(m2.any_field_assigned_with_prefix_or_suffix(&[], &["_audited"]));
    }

    #[test]
    fn any_field_accessed_with_suffix_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![Node::ExpressionStatement {
                expr: Box::new(field_access("sensor", "read_at")),
                span: span::Span::default(),
            }],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_field_accessed_with_suffix(&["_at"]));
        assert!(!m.any_field_accessed_with_suffix(&["_zzz"]));
    }

    #[test]
    fn any_call_ident_with_suffix_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![Node::ExpressionStatement {
                expr: Box::new(call("file_exists")),
                span: span::Span::default(),
            }],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_call_ident_with_suffix(&["_exists", "_is_valid"]));
        assert!(!m.any_call_ident_with_suffix(&["_zzz"]));
    }

    #[test]
    fn any_call_ident_containing_matches() {
        let program = Node::Program(vec![fn_with_body(
            "fixture",
            vec![],
            vec![Node::ExpressionStatement {
                expr: Box::new(call("read_epoch3")),
                span: span::Span::default(),
            }],
        )]);
        let m = Markers::scan(&program);
        assert!(m.any_call_ident_containing(&["_epoch"]));
        assert!(!m.any_call_ident_containing(&["_zzz"]));
    }
}
