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
use std::sync::OnceLock;
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct RefinementSpec {
    pub name: String,
    pub base: String,
    pub predicate: String,
}

static REFINEMENTS: RwLock<Vec<RefinementSpec>> = RwLock::new(Vec::new());

/// RES-3839: Process-wide flag controlling strict refinement type checking.
/// When true, unresolved (Z3 Unknown) refinement obligations become hard
/// errors instead of warnings. Set via `--strict-refinements` CLI flag.
static STRICT_REFINEMENTS: OnceLock<bool> = OnceLock::new();

pub fn set_strict_refinements(strict: bool) {
    // Ignore the error if already set (permits re-initialization in tests).
    let _ = STRICT_REFINEMENTS.set(strict);
}

fn is_strict_refinements() -> bool {
    STRICT_REFINEMENTS.get().copied().unwrap_or(false)
}

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
    // Build a name-indexed map for O(1) annotation lookups before
    // install() moves the Vec.
    let spec_map: std::collections::HashMap<String, RefinementSpec> =
        specs.iter().map(|s| (s.name.clone(), s.clone())).collect();
    install(specs);
    // Compile-time predicate verification: scan for `let x: Refined = CONST`
    // bindings and reject those whose constant value violates the predicate.
    check_let_obligations(program, source_path, &spec_map)
}

/// Walk the program and emit a compile-time error for any `let` binding
/// whose type annotation is a refinement type and whose RHS is an integer
/// literal that violates the refinement predicate, or (with Z3 enabled) whose
/// RHS is a parameter reference and the refinement predicate cannot be proven
/// from the enclosing function's `requires` clauses.
///
/// For non-literal RHS values (arbitrary expressions, external I/O) this pass
/// is a no-op — runtime `refine_int` guards enforce the predicate at
/// execution time.
fn check_let_obligations(
    program: &Node,
    source_path: &str,
    specs: &std::collections::HashMap<String, RefinementSpec>,
) -> Result<(), String> {
    // Walk only the top-level function bodies — LetStatements inside
    // function bodies are the primary refinement obligation sites.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for stmt in stmts {
        check_node_obligations(&stmt.node, source_path, specs, None)?;
    }
    Ok(())
}

/// Context of the enclosing function (if any) for checking refinement
/// obligations on parameter references. RES-3839: used by the Z3 path
/// to discharge refinement predicates against function `requires` clauses.
#[derive(Clone)]
struct FunctionContext {
    /// Parameter names in order: parameters[i].1 is the i-th parameter name.
    parameters: Vec<(String, String)>, // (type, name)
    /// Precondition clauses that constrain the parameters.
    requires: Vec<Node>,
}

/// RES-3839: Attempt to prove a refinement predicate on a parameter using Z3.
/// Returns Err only when the predicate is disproven or when Z3 returns Unknown
/// and strict mode is enabled. Warnings (Unknown case, non-strict) are logged
/// but do not stop compilation.
#[cfg(feature = "z3")]
fn check_parameter_refinement_with_z3(
    param_name: &str,
    refinement_name: &str,
    spec: &RefinementSpec,
    source_path: &str,
    span: &crate::span::Span,
    binding_name: &str,
    axioms: &[Node],
) -> Result<(), String> {
    // Convert the predicate string into an AST node, substituting the parameter name for "self".
    let predicate_expr = match build_predicate_node(param_name, &spec.predicate) {
        Some(expr) => expr,
        None => {
            // Predicate is not in a form Z3 can handle; skip this check.
            return Ok(());
        }
    };

    // Use a reasonable timeout for refinement checks (same as contract clauses).
    const REFINEMENT_TIMEOUT_MS: u32 = 1000;

    let (verdict, _cert, counterexample, _timed_out) =
        crate::verifier_z3::prove_with_axioms_and_timeout(
            &predicate_expr,
            &std::collections::HashMap::new(),
            axioms,
            REFINEMENT_TIMEOUT_MS,
        );

    let line = span.start.line;
    match verdict {
        Some(true) => {
            // Proved: refinement is satisfied, no error.
            Ok(())
        }
        Some(false) => {
            // Disproved: refinement cannot hold for this parameter given the requires clauses.
            let cx_str = counterexample
                .map(|c| format!(" (counterexample: {})", c))
                .unwrap_or_default();
            Err(format!(
                "{}:{}: refinement error: let `{}`: {} of type `{}` cannot satisfy the refinement predicate{}",
                source_path, line, binding_name, param_name, refinement_name, cx_str
            ))
        }
        None => {
            // Unknown: Z3 couldn't decide.
            if is_strict_refinements() {
                Err(format!(
                    "{}:{}: refinement error: let `{}`: static verification inconclusive for `{}` under `--strict-refinements`",
                    source_path, line, binding_name, refinement_name
                ))
            } else {
                // Issue a warning but allow compilation to proceed.
                eprintln!(
                    "Warning: {}:{}: refinement type `{}` on parameter `{}` (bound to `{}`) could not be statically verified; runtime check will be applied",
                    source_path, line, refinement_name, param_name, binding_name
                );
                Ok(())
            }
        }
    }
}

/// Convert a simple refinement predicate string into an AST node.
/// Replaces "self" with the given parameter name.
/// Returns None if the predicate is not in the simple form: "SELF OP INT".
fn build_predicate_node(param_name: &str, predicate: &str) -> Option<Node> {
    let p = predicate.trim();
    let mut parts = p.split_whitespace();
    let lhs = parts.next()?;
    let op_str = parts.next()?;
    let rhs_str = parts.next()?;

    // Only support three-part predicates: SELF OP INT.
    if parts.next().is_some() {
        return None;
    }

    if lhs != "self" {
        return None;
    }

    // Map the operator string to a static &'static str.
    // These are the comparison operators supported by the Z3 translator.
    let operator: &'static str = match op_str {
        ">" => ">",
        "<" => "<",
        ">=" => ">=",
        "<=" => "<=",
        "==" => "==",
        "!=" => "!=",
        _ => return None,
    };

    let rhs_val: i64 = rhs_str.parse().ok()?;

    let sp = crate::span::Span::default();
    Some(Node::InfixExpression {
        left: Box::new(Node::Identifier {
            name: param_name.to_string(),
            span: sp,
        }),
        operator,
        right: Box::new(Node::IntegerLiteral {
            value: rhs_val,
            span: sp,
        }),
        span: sp,
    })
}

fn check_node_obligations(
    node: &Node,
    source_path: &str,
    specs: &std::collections::HashMap<String, RefinementSpec>,
    fn_ctx: Option<FunctionContext>,
) -> Result<(), String> {
    // Handle the Program node specially to extract statement nodes.
    if let Node::Program(stmts) = node {
        for stmt in stmts {
            check_node_obligations(&stmt.node, source_path, specs, fn_ctx.clone())?;
        }
        return Ok(());
    }

    match node {
        Node::LetStatement {
            name,
            type_annot: Some(ty_name),
            value,
            span,
            ..
        } => {
            if let Some(spec) = specs.get(ty_name.as_str()) {
                match value.as_ref() {
                    // Check constant integer literals at compile time.
                    Node::IntegerLiteral { value: int_val, .. } => {
                        if let Err(msg) = evaluate_int_predicate(*int_val, spec) {
                            let line = span.start.line;
                            return Err(format!(
                                "{}:{}: refinement error: let `{}`: {}",
                                source_path, line, name, msg
                            ));
                        }
                    }
                    // RES-3839: with Z3 support, check parameter references using requires clauses.
                    #[cfg(feature = "z3")]
                    Node::Identifier {
                        name: param_name, ..
                    } if fn_ctx.is_some() => {
                        let ctx = fn_ctx.as_ref().unwrap();
                        // Check if param_name is one of the function's parameters.
                        if ctx.parameters.iter().any(|(_, pname)| pname == param_name) {
                            // Try to prove the refinement predicate holds for this parameter
                            // using the function's requires clauses as axioms.
                            check_parameter_refinement_with_z3(
                                param_name,
                                ty_name,
                                spec,
                                source_path,
                                span,
                                name,
                                &ctx.requires,
                            )?;
                        }
                    }
                    _ => {}
                }
                // Recurse into the value expression.
                check_node_obligations(value, source_path, specs, fn_ctx.clone())?;
            }
        }
        Node::Function {
            body,
            parameters,
            requires,
            ..
        } => {
            // Extract function context for recursive checks in the body.
            let fn_context = FunctionContext {
                parameters: parameters.clone(),
                requires: requires.clone(),
            };
            check_node_obligations(body, source_path, specs, Some(fn_context))?;
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                check_node_obligations(s, source_path, specs, fn_ctx.clone())?;
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            check_node_obligations(expr, source_path, specs, fn_ctx)?;
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            check_node_obligations(condition, source_path, specs, fn_ctx.clone())?;
            check_node_obligations(consequence, source_path, specs, fn_ctx.clone())?;
            if let Some(alt) = alternative {
                check_node_obligations(alt, source_path, specs, fn_ctx)?;
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            check_node_obligations(condition, source_path, specs, fn_ctx.clone())?;
            check_node_obligations(body, source_path, specs, fn_ctx)?;
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            check_node_obligations(v, source_path, specs, fn_ctx)?;
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

    fn make_spec_map(
        name: &str,
        predicate: &str,
    ) -> std::collections::HashMap<String, RefinementSpec> {
        let mut m = std::collections::HashMap::new();
        m.insert(
            name.to_string(),
            RefinementSpec {
                name: name.to_string(),
                base: "int".to_string(),
                predicate: predicate.to_string(),
            },
        );
        m
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
            is_const: false,
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
        let specs = make_spec_map("Positive", "self > 0");
        let program = make_let("Positive", 5);
        assert!(check_node_obligations(&program, "test.rz", &specs, None).is_ok());
    }

    #[test]
    fn check_let_obligation_fails_for_invalid_value() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs = make_spec_map("Positive", "self > 0");
        let program = make_let("Positive", -1);
        let result = check_node_obligations(&program, "test.rz", &specs, None);
        assert!(result.is_err(), "expected error, got Ok");
        let msg = result.unwrap_err();
        assert!(msg.contains("refinement error"), "unexpected msg: {}", msg);
        assert!(msg.contains("Positive"), "missing type name: {}", msg);
    }

    #[test]
    fn check_let_obligation_ignores_unknown_type_annotations() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs = make_spec_map("Positive", "self > 0");
        // `let x: MyStruct = 0` — MyStruct is not a refinement type, should pass.
        let program = make_let("MyStruct", 0);
        assert!(check_node_obligations(&program, "test.rz", &specs, None).is_ok());
    }

    #[test]
    fn check_empty_program_is_noop() {
        let _g = crate::feature_attrs::lock_for_test();
        let specs = make_spec_map("Positive", "self > 0");
        let program = Node::Program(vec![]);
        assert!(check_node_obligations(&program, "test.rz", &specs, None).is_ok());
    }

    // ── Z3-backed checks (RES-3839) ────────────────────────────────────

    #[cfg(feature = "z3")]
    #[test]
    fn z3_proves_parameter_within_bounds() {
        let _g = crate::feature_attrs::lock_for_test();
        set_strict_refinements(false);
        let specs = make_spec_map("Positive", "self > 0");

        // Build a program: `fn f(int x) requires x > 0 { let y: Positive = x; }`
        let sp = crate::span::Span::default();
        let requires_clause = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: sp,
            }),
            operator: ">",
            right: Box::new(Node::IntegerLiteral { value: 0, span: sp }),
            span: sp,
        };

        let let_stmt = Node::LetStatement {
            name: "y".to_string(),
            value: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: sp,
            }),
            type_annot: Some("Positive".to_string()),
            span: sp,
            is_const: false,
        };

        let body = Node::Block {
            stmts: vec![let_stmt],
            span: sp,
        };

        let func = Node::Function {
            name: "f".to_string(),
            parameters: vec![("int".to_string(), "x".to_string())],
            defaults: vec![],
            body: Box::new(body),
            requires: vec![requires_clause],
            ensures: vec![],
            return_type: None,
            span: sp,
            pure: false,
            effects: Default::default(),
            type_params: vec![],
            type_param_bounds: vec![],
            fails: vec![],
            recovers_to: None,
            is_pub: false,
        };

        let program = Node::Program(vec![span::Spanned::new(func, sp)]);
        let result = check_node_obligations(&program, "test.rz", &specs, None);
        assert!(result.is_ok(), "expected proved case to pass: {:?}", result);
    }

    #[cfg(feature = "z3")]
    #[test]
    fn z3_disproves_parameter_violating_bounds() {
        let _g = crate::feature_attrs::lock_for_test();
        set_strict_refinements(false);
        let specs = make_spec_map("Positive", "self > 0");

        // Build a program: `fn f(int x) requires x <= 0 { let y: Positive = x; }`
        // The requires clause contradicts the refinement, so Z3 should disprove it.
        let sp = crate::span::Span::default();
        let requires_clause = Node::InfixExpression {
            left: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: sp,
            }),
            operator: "<=",
            right: Box::new(Node::IntegerLiteral { value: 0, span: sp }),
            span: sp,
        };

        let let_stmt = Node::LetStatement {
            name: "y".to_string(),
            value: Box::new(Node::Identifier {
                name: "x".to_string(),
                span: sp,
            }),
            type_annot: Some("Positive".to_string()),
            span: sp,
            is_const: false,
        };

        let body = Node::Block {
            stmts: vec![let_stmt],
            span: sp,
        };

        let func = Node::Function {
            name: "f".to_string(),
            parameters: vec![("int".to_string(), "x".to_string())],
            defaults: vec![],
            body: Box::new(body),
            requires: vec![requires_clause],
            ensures: vec![],
            return_type: None,
            span: sp,
            pure: false,
            effects: Default::default(),
            type_params: vec![],
            type_param_bounds: vec![],
            fails: vec![],
            recovers_to: None,
            is_pub: false,
        };

        let program = Node::Program(vec![span::Spanned::new(func, sp)]);
        let result = check_node_obligations(&program, "test.rz", &specs, None);
        assert!(result.is_err(), "expected disproved case to fail");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("refinement error"),
            "expected 'refinement error' in message: {}",
            msg
        );
    }
}
