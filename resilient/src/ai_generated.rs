//! `@ai_generated` validation.
//!
//! AI-generated functions must carry **non-vacuous** contracts:
//!
//! 1. Both `requires` AND `ensures` must be present — a function needs both a
//!    precondition (what the caller guarantees) and a postcondition (what the
//!    function guarantees back). Either alone is insufficient for Z3 to prove
//!    correctness.
//!
//! 2. Every `requires` clause must reference at least one parameter — `requires
//!    true` is vacuous and gives the verifier nothing to constrain.
//!
//! 3. Every `ensures` clause must reference `result` — `ensures x > 0` re-
//!    states the input guard rather than constraining the output. An ensures
//!    clause that does not mention `result` cannot be used to prove the
//!    postcondition of the function.
//!
//! These rules together ensure that `@ai_generated` annotations carry real,
//! checkable specifications rather than rubber-stamp boilerplate.

use std::collections::HashMap;

use crate::Node;

fn diagnostic(source_path: &str, line: usize, fn_name: &str, message: &str) -> String {
    format!(
        "{source_path}:{line}:0: error[ai_generated]: invalid @ai_generated declaration `{fn_name}`: {message}"
    )
}

/// Returns true if the expression tree contains an `Identifier` with the given name.
fn expr_references(node: &Node, name: &str) -> bool {
    match node {
        Node::Identifier { name: n, .. } => n == name,
        Node::InfixExpression { left, right, .. } => {
            expr_references(left, name) || expr_references(right, name)
        }
        Node::PrefixExpression { right, .. } => expr_references(right, name),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => expr_references(function, name) || arguments.iter().any(|a| expr_references(a, name)),
        Node::IndexExpression { target, index, .. } => {
            expr_references(target, name) || expr_references(index, name)
        }
        Node::FieldAccess { target, .. } => expr_references(target, name),
        _ => false,
    }
}

/// Returns true if the expression is a trivial vacuous literal (`true`).
fn is_vacuous(node: &Node) -> bool {
    matches!(node, Node::BooleanLiteral { value: true, .. })
}

/// Collected contract info for one function.
///
/// RES-3854: shared with [`crate::contract_policy`], which applies the
/// same clause-vacuity rules to functions enrolled by the module-level
/// `@require_contracts` directive rather than the `@ai_generated` tag.
pub(crate) struct ContractInfo {
    pub(crate) params: Vec<String>,
    pub(crate) requires: Vec<Node>,
    pub(crate) ensures: Vec<Node>,
    pub(crate) line: usize,
}

/// RES-3854: clause-vacuity rules 2 and 3, shared between the
/// `@ai_generated` pass and `contract_policy`. Returns bare messages
/// (no source position / error-code prefix); each caller wraps them in
/// its own diagnostic format. Only *declared* clauses are judged —
/// presence of `requires`/`ensures` is each caller's own policy.
pub(crate) fn contract_clause_errors(info: &ContractInfo) -> Vec<String> {
    let mut errors = Vec::new();
    for clause in &info.requires {
        if is_vacuous(clause) {
            errors.push(
                "`requires true` is a vacuous precondition — write a real constraint \
                 on the function's parameters (e.g. `requires n >= 0`)"
                    .to_string(),
            );
        } else if !info.params.is_empty() && !info.params.iter().any(|p| expr_references(clause, p))
        {
            errors.push(
                "`requires` clause does not reference any function parameter — \
                 preconditions must constrain the input domain"
                    .to_string(),
            );
        }
    }
    for clause in &info.ensures {
        if is_vacuous(clause) {
            errors.push(
                "`ensures true` is a vacuous postcondition — write a real constraint \
                 on `result` (e.g. `ensures result >= 0`)"
                    .to_string(),
            );
        } else if !expr_references(clause, "result") {
            errors.push(
                "`ensures` clause does not reference `result` — postconditions must \
                 constrain the return value (e.g. `ensures result >= 0`)"
                    .to_string(),
            );
        }
    }
    errors
}

pub(crate) fn collect_contracts(program: &Node) -> HashMap<String, ContractInfo> {
    let mut map = HashMap::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                parameters,
                requires,
                ensures,
                span,
                ..
            } = &stmt.node
            {
                let param_names: Vec<String> = parameters.iter().map(|(_, n)| n.clone()).collect();
                map.insert(
                    name.clone(),
                    ContractInfo {
                        params: param_names,
                        requires: requires.clone(),
                        ensures: ensures.clone(),
                        line: span.start.line,
                    },
                );
            }
        }
    }
    map
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("ai_generated");
    if attrs.is_empty() {
        return Ok(());
    }

    let contracts = collect_contracts(program);
    let mut errors: Vec<String> = Vec::new();

    for (fn_name, _rec) in &attrs {
        let Some(info) = contracts.get(fn_name) else {
            continue;
        };

        let line = info.line;

        // Rule 1a: must have at least one `requires` clause.
        if info.requires.is_empty() {
            errors.push(diagnostic(
                source_path,
                line,
                fn_name,
                "`@ai_generated` function must declare at least one `requires` clause \
                 constraining its inputs — add `requires <param_condition>`",
            ));
        }

        // Rule 1b: must have at least one `ensures` clause.
        if info.ensures.is_empty() {
            errors.push(diagnostic(
                source_path,
                line,
                fn_name,
                "`@ai_generated` function must declare at least one `ensures` clause \
                 constraining its output — add `ensures result <condition>`",
            ));
        }

        // Rules 2 & 3: declared clauses must be non-vacuous (shared with
        // contract_policy — see contract_clause_errors).
        for msg in contract_clause_errors(info) {
            errors.push(diagnostic(source_path, line, fn_name, &msg));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_and_check(src: &str) -> Result<(), String> {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let result = check(&prog, "<test>");
        crate::feature_attrs::reset();
        result
    }

    #[test]
    fn parses_at_ai_generated_attribute() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "@ai_generated\nfn made_by_model(int x) requires x >= 0 ensures result >= 0 { return x + 1; }";
        let (_prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let attrs = crate::feature_attrs::find_kind("ai_generated");
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].0, "made_by_model");
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_ai_generated_function_without_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "made_by_model",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: "".into(),
                line: 1,
            },
        );
        let src = "fn made_by_model(int x) -> int { return x + 1; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let err = check(&prog, "<test>").expect_err("missing contracts must be rejected");
        assert!(
            err.contains("error[ai_generated]"),
            "unexpected error: {err}"
        );
        assert!(
            err.contains("requires") || err.contains("ensures"),
            "should mention missing clause: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_vacuous_requires_true() {
        let err = parse_and_check(
            "@ai_generated\nfn f(int x) requires true ensures result >= 0 { return x; }",
        )
        .expect_err("vacuous requires must be rejected");
        assert!(err.contains("vacuous precondition"), "unexpected: {err}");
    }

    #[test]
    fn rejects_vacuous_ensures_true() {
        let err = parse_and_check(
            "@ai_generated\nfn f(int x) requires x >= 0 ensures true { return x; }",
        )
        .expect_err("vacuous ensures must be rejected");
        assert!(err.contains("vacuous postcondition"), "unexpected: {err}");
    }

    #[test]
    fn rejects_ensures_without_result() {
        let err = parse_and_check(
            "@ai_generated\nfn f(int x) requires x >= 0 ensures x >= 0 { return x; }",
        )
        .expect_err("ensures without result must be rejected");
        assert!(
            err.contains("does not reference `result`"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn rejects_requires_without_param() {
        let err = parse_and_check(
            "@ai_generated\nfn f(int x) requires 1 > 0 ensures result >= 0 { return x; }",
        )
        .expect_err("requires without param must be rejected");
        assert!(
            err.contains("does not reference any function parameter"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn accepts_real_contracts() {
        assert!(
            parse_and_check(
                "@ai_generated\nfn f(int x) requires x >= 0 ensures result >= x { return x + 1; }"
            )
            .is_ok()
        );
    }

    #[test]
    fn accepts_multiple_real_clauses() {
        assert!(parse_and_check(
            "@ai_generated\nfn add(int a, int b) requires a >= 0 requires b >= 0 ensures result >= a ensures result >= b { return a + b; }"
        )
        .is_ok());
    }

    #[test]
    fn typechecker_rejects_at_ai_generated_without_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "@ai_generated\nfn made_by_model(int x) -> int { return x + 1; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = crate::typechecker::TypeChecker::new();
        let err = tc
            .check_program_with_source(&prog, "<test>")
            .expect_err("typecheck must reject @ai_generated without contracts");
        assert!(
            err.contains("error[ai_generated]"),
            "expected ai_generated diagnostic, got: {err}"
        );
        crate::feature_attrs::reset();
    }
}
