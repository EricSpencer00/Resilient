//! Feature 12/50 — Refinement Types.
//!
//! `#[refinement(name = "PositiveInt", base = "int", where = "self > 0")]`
//! attached to a `type` alias creates a refinement type: a base type
//! constrained by a Z3-checkable predicate. Unlike a runtime contract,
//! the refinement is part of the type — assigning a value to a
//! refined variable triggers a Z3 obligation that the value satisfies
//! the predicate.
//!
//! This first slice records refinement specs in a process-wide
//! registry. The typechecker integration (call site → obligation) is
//! a downstream PR; what ships here is:
//!
//! 1. The attribute parser (via `feature_attrs`).
//! 2. The spec registry — `RefinementSpec { name, base, predicate }`.
//! 3. A `refine(value, refinement_name)` runtime guard helper that
//!    can be called from generated code (used by tests today).
//! 4. A `--list-refinements` audit surface.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct RefinementSpec {
    pub name: String,
    pub base: String,
    pub predicate: String,
}

static REFINEMENTS: RwLock<Vec<RefinementSpec>> = RwLock::new(Vec::new());

pub fn collect_specs() -> Vec<RefinementSpec> {
    let attrs = crate::feature_attrs::find_kind("refinement");
    // RES-1754: pre-size to attrs.len() — exactly one push per
    // attribute record (no conditional skip), so this is an exact
    // bound, not an over-estimate. Same shape as the pre-size series
    // (RES-1742…RES-1752).
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = RefinementSpec {
            name: item,
            base: String::new(),
            predicate: String::new(),
        };
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "base" | "where" => {
                        if k == "base" {
                            spec.base = v.to_string();
                        } else {
                            spec.predicate = v.to_string();
                        }
                    }
                    "name" => {
                        spec.name = v.to_string();
                    }
                    _ => {}
                }
            }
        }
        out.push(spec);
    }
    out
}

pub fn install(specs: Vec<RefinementSpec>) {
    if let Ok(mut g) = REFINEMENTS.write() {
        *g = specs;
    }
}

pub fn lookup(name: &str) -> Option<RefinementSpec> {
    REFINEMENTS
        .read()
        .ok()
        .and_then(|g| g.iter().find(|s| s.name == name).cloned())
}

/// Trivial runtime guard: evaluates a refinement predicate against an
/// integer. The predicate language is intentionally tiny: the literal
/// `self`, an operator (`>`, `<`, `>=`, `<=`, `==`, `!=`), and an
/// integer literal. Anything more complex falls back to "satisfied"
/// and the Z3 path takes over (downstream PR).
/// Evaluate a refinement predicate directly from a `RefinementSpec`,
/// without consulting the global REFINEMENTS registry. Used by the
/// compile-time obligation checker so it can work off a local spec_map.
pub(crate) fn evaluate_int_predicate(value: i64, spec: &RefinementSpec) -> Result<i64, String> {
    let p = spec.predicate.trim();
    let mut parts = p.split_whitespace();
    let lhs = parts.next().unwrap_or("self");
    let op = parts.next().unwrap_or("==");
    let rhs: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if lhs != "self" {
        return Ok(value);
    }
    let ok = match op {
        ">" => value > rhs,
        "<" => value < rhs,
        ">=" => value >= rhs,
        "<=" => value <= rhs,
        "==" => value == rhs,
        "!=" => value != rhs,
        _ => true,
    };
    if ok {
        Ok(value)
    } else {
        Err(format!(
            "refinement `{}` violated: {} {} {} is false",
            spec.name, value, op, rhs
        ))
    }
}

pub fn refine_int(value: i64, refinement_name: &str) -> Result<i64, String> {
    let spec = match lookup(refinement_name) {
        Some(s) => s,
        None => return Ok(value), // unknown refinement: pass through
    };
    let p = spec.predicate.trim();
    let mut parts = p.split_whitespace();
    let lhs = parts.next().unwrap_or("self");
    let op = parts.next().unwrap_or("==");
    let rhs: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if lhs != "self" {
        return Ok(value);
    }
    let ok = match op {
        ">" => value > rhs,
        "<" => value < rhs,
        ">=" => value >= rhs,
        "<=" => value <= rhs,
        "==" => value == rhs,
        "!=" => value != rhs,
        _ => true,
    };
    if ok {
        Ok(value)
    } else {
        Err(format!(
            "refinement `{}` violated: {} {} {} is false",
            spec.name, value, op, rhs
        ))
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    // RES-1302: skip the `install` call when the current program
    // declares no `#[refinement]` attributes. The `install` helper
    // *replaces* the process-global `REFINEMENTS` vector — calling
    // it with an empty list wipes whatever the previous compilation
    // (or, in `cargo test`, a parallel test that called `install`
    // directly under `feature_attrs::lock_for_test()`) set.
    //
    // The race: `refine_int_enforces_predicate` holds
    // `feature_attrs::lock_for_test()` and calls `install([Positive])`
    // directly. That lock guards `ATTR_REGISTRY`, not `REFINEMENTS`.
    // A parallel test that parses + typechecks a program with no
    // refinement attribute lands here, where `collect_specs()`
    // returns `[]`, and the unconditional `install(vec![])` clears
    // `REFINEMENTS` between the test's install and its
    // `refine_int(value, "Positive")` lookup. The lookup then
    // returns `None`, `refine_int` passes the value through, and
    // the `assert!(refine_int(-1, "Positive").is_err())` assertion
    // fails. Skipping the call when the input is empty avoids the
    // wipe; production compilation only mutates the global when the
    // *current* program declares refinements, which is the only
    // case the global is needed for that program anyway.
    let specs = collect_specs();
    if specs.is_empty() {
        return Ok(());
    }
    // RES-2374: build the lookup map as `&str → &RefinementSpec`
    // borrows into `specs` before calling `install`, then run the
    // check and install afterward. The previous shape cloned each
    // RefinementSpec twice (once for the key, once for the value —
    // and RefinementSpec carries three Strings) to satisfy the
    // ordering constraint that `install(specs)` moves the Vec before
    // the check ran. By scoping the borrowed map and dropping it at
    // end of block, install can take ownership of `specs` next.
    //
    // Same install-after-validate shape as
    // `recursive_types::check` (RES-1485), `ghost_types::check` /
    // `async_await::check` (RES-1487), and
    // `distributed_invariants::check` (RES-1491).
    let result = {
        let spec_map: std::collections::HashMap<&str, &RefinementSpec> =
            specs.iter().map(|s| (s.name.as_str(), s)).collect();
        check_let_obligations(program, source_path, &spec_map)
    };
    install(specs);
    result
}

/// Walk the program and emit a compile-time error for any `let` binding
/// whose type annotation is a refinement type and whose RHS is an integer
/// literal that violates the refinement predicate.
///
/// For non-literal RHS values this pass is a no-op — runtime `refine_int`
/// guards enforce the predicate at execution time; Z3-backed static
/// verification of symbolic values is a downstream PR.
fn check_let_obligations(
    program: &Node,
    source_path: &str,
    specs: &std::collections::HashMap<&str, &RefinementSpec>,
) -> Result<(), String> {
    // Walk only the top-level function bodies — LetStatements inside
    // function bodies are the primary refinement obligation sites.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for stmt in stmts {
        check_node_obligations(&stmt.node, source_path, specs)?;
    }
    Ok(())
}

fn check_node_obligations(
    node: &Node,
    source_path: &str,
    specs: &std::collections::HashMap<&str, &RefinementSpec>,
) -> Result<(), String> {
    match node {
        Node::LetStatement {
            name,
            type_annot: Some(ty_name),
            value,
            span,
            ..
        } => {
            if let Some(spec) = specs.get(ty_name.as_str()) {
                // Only check constant integer literals for now.
                if let Node::IntegerLiteral { value: int_val, .. } = value.as_ref() {
                    if let Err(msg) = evaluate_int_predicate(*int_val, spec) {
                        let line = span.start.line;
                        return Err(format!(
                            "{}:{}: refinement error: let `{}`: {}",
                            source_path, line, name, msg
                        ));
                    }
                }
                // Recurse into the value expression.
                check_node_obligations(value, source_path, specs)?;
            }
        }
        Node::Function { body, .. } => check_node_obligations(body, source_path, specs)?,
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_node_obligations(s, source_path, specs)?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            check_node_obligations(expr, source_path, specs)?;
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_node_obligations(condition, source_path, specs)?;
            check_node_obligations(consequence, source_path, specs)?;
            if let Some(alt) = alternative {
                check_node_obligations(alt, source_path, specs)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_node_obligations(condition, source_path, specs)?;
            check_node_obligations(body, source_path, specs)?;
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            check_node_obligations(v, source_path, specs)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span;

    #[test]
    fn collects_refinement_spec() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "PositiveInt",
            crate::feature_attrs::AttrRecord {
                name: "refinement".into(),
                args: r#"base = "int", where = "self > 0""#.into(),
                line: 0,
            },
        );
        let specs = collect_specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "PositiveInt");
        assert_eq!(specs[0].predicate, "self > 0");
        crate::feature_attrs::reset();
    }

    #[test]
    fn refine_int_enforces_predicate() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![RefinementSpec {
            name: "Positive".into(),
            base: "int".into(),
            predicate: "self > 0".into(),
        }]);
        assert!(refine_int(5, "Positive").is_ok());
        assert!(refine_int(0, "Positive").is_err());
        assert!(refine_int(-1, "Positive").is_err());
    }

    #[test]
    fn unknown_refinement_passes_through() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        install(vec![]);
        assert_eq!(refine_int(42, "DoesntExist").ok(), Some(42));
    }

    // ── compile-time obligation checks ─────────────────────────────────────

    fn make_specs(name: &str, predicate: &str) -> Vec<RefinementSpec> {
        vec![RefinementSpec {
            name: name.to_string(),
            base: "int".to_string(),
            predicate: predicate.to_string(),
        }]
    }

    fn borrow_specs(specs: &[RefinementSpec]) -> std::collections::HashMap<&str, &RefinementSpec> {
        specs.iter().map(|s| (s.name.as_str(), s)).collect()
    }

    fn make_let(ty_name: &str, int_val: i64) -> Node {
        let let_node = Node::LetStatement {
            name: "x".into(),
            value: Box::new(Node::IntegerLiteral {
                value: int_val,
                span: Default::default(),
            }),
            type_annot: Some(ty_name.to_string()),
            span: Default::default(),
        };
        // Wrap in a Block so the traversal descends into it.
        let block = Node::Block {
            stmts: vec![let_node],
            span: Default::default(),
        };
        // Wrap in a Program statement directly to avoid constructing
        // the full Node::Function with its many required fields.
        Node::Program(vec![span::Spanned::new(block, Default::default())])
    }

    #[test]
    fn check_let_obligation_passes_for_valid_value() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs_vec = make_specs("Positive", "self > 0");
        let specs = borrow_specs(&specs_vec);
        let program = make_let("Positive", 5);
        assert!(check_let_obligations(&program, "test.rz", &specs).is_ok());
    }

    #[test]
    fn check_let_obligation_fails_for_invalid_value() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs_vec = make_specs("Positive", "self > 0");
        let specs = borrow_specs(&specs_vec);
        let program = make_let("Positive", -1);
        let result = check_let_obligations(&program, "test.rz", &specs);
        assert!(result.is_err(), "expected error, got Ok");
        let msg = result.unwrap_err();
        assert!(msg.contains("refinement error"), "unexpected msg: {}", msg);
        assert!(msg.contains("Positive"), "missing type name: {}", msg);
    }

    #[test]
    fn check_let_obligation_ignores_unknown_type_annotations() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs_vec = make_specs("Positive", "self > 0");
        let specs = borrow_specs(&specs_vec);
        // `let x: MyStruct = 0` — MyStruct is not a refinement type, should pass.
        let program = make_let("MyStruct", 0);
        assert!(check_let_obligations(&program, "test.rz", &specs).is_ok());
    }

    #[test]
    fn check_empty_program_is_noop() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs_vec = make_specs("Positive", "self > 0");
        let specs = borrow_specs(&specs_vec);
        let program = Node::Program(vec![]);
        assert!(check_let_obligations(&program, "test.rz", &specs).is_ok());
    }
}
