//! RES-3854: provenance-agnostic contract policy — `@require_contracts`.
//!
//! Verification is a property of the *code*, not of who wrote it. The
//! module-level `@require_contracts` directive enrols every function in
//! the file into the Tier 1 non-vacuous-contract checks that were
//! previously reachable only through the `@ai_generated` provenance tag
//! (see `resilient/src/ai_generated.rs`):
//!
//! * every declared `requires` clause must reference at least one
//!   function parameter (`requires true` is rejected as vacuous);
//! * every declared `ensures` clause must reference `result`
//!   (`ensures true` and input-only restatements are rejected).
//!
//! Enrolment is a *policy* decision made once for the module — nobody
//! can silently opt a function out by deleting a per-function
//! annotation. The enrolment predicate [`is_enrolled`] is the single
//! source of truth shared by downstream verification passes (RES-3857
//! drives Tier 2 loop-bound verification from it).
//!
//! ## Provenance grants no powers (RES-3858)
//!
//! The `@ai_generated` tag is a pure provenance alias of the
//! `#[generated]` annotation: it records audit metadata and nothing
//! else. Adding or removing it changes no diagnostic — enrolment comes
//! only from this module's `@require_contracts` policy. Under the bare
//! directive, functions with no contract clauses at all are accepted.
//!
//! ## Strict policy — `@require_contracts(strict)`
//!
//! The strict variant additionally mandates contract *presence*: every
//! named function (except `main`) must declare at least one `ensures`
//! clause, and at least one `requires` clause when it has parameters.
//! This is the "safety-critical crate" posture from #3854: nobody can
//! opt a function out of verification by simply not writing a
//! contract.

use crate::Node;

/// Registry key the parser records the module-level directive under.
/// The directive attaches to the file, not to any item, so it uses a
/// reserved pseudo-item name no real function can shadow.
pub(crate) const MODULE_KEY: &str = "<module>";

/// True when the current file declared `@require_contracts`.
pub(crate) fn module_requires_contracts() -> bool {
    !crate::feature_attrs::find_kind("require_contracts").is_empty()
}

/// RES-3854 strict policy: `@require_contracts(strict)` additionally
/// mandates contract *presence* — every named function must carry a
/// non-vacuous `ensures` clause (and a `requires` clause when it has
/// parameters). `main` is exempt: it is the program entry point, takes
/// no caller-visible inputs, and returns no verifiable `result`.
pub(crate) fn strict_mode() -> bool {
    crate::feature_attrs::find_kind("require_contracts")
        .iter()
        .any(|(_, rec)| rec.args.trim() == "strict")
}

/// RES-3854 enrolment predicate: is `fn_name` subject to contract
/// verification? True only under the module-level `@require_contracts`
/// directive — provenance tags (`@ai_generated`, `#[generated]`) grant
/// no verification behaviour (RES-3858). Downstream verification
/// passes (Tier 2 loop bounds, proof certificates) share this single
/// source of truth.
pub(crate) fn is_enrolled(fn_name: &str) -> bool {
    let _ = fn_name;
    module_requires_contracts()
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
pub(crate) struct ContractInfo {
    pub(crate) params: Vec<String>,
    pub(crate) requires: Vec<Node>,
    pub(crate) ensures: Vec<Node>,
    pub(crate) line: usize,
}

pub(crate) fn collect_contracts(program: &Node) -> std::collections::HashMap<String, ContractInfo> {
    let mut map = std::collections::HashMap::new();
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

/// Tier 1 clause-vacuity rules 2 and 3. Returns bare messages (no
/// source position / error-code prefix). Only *declared* clauses are
/// judged — presence is the strict policy's job.
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

fn diagnostic(source_path: &str, line: usize, fn_name: &str, message: &str) -> String {
    format!(
        "{source_path}:{line}:0: error[contract_policy]: function `{fn_name}` violates `@require_contracts`: {message}"
    )
}

/// Typecheck pass: apply Tier 1 clause-vacuity rules to every enrolled,
/// contract-carrying function. Called from `typechecker.rs`
/// `<EXTENSION_PASSES>`.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let contracts = collect_contracts(program);
    let strict = strict_mode();
    let mut errors: Vec<String> = Vec::new();

    // Iterate the program (not the contract map) so diagnostics come
    // out in source order deterministically.
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            let Node::Function { name, .. } = &stmt.node else {
                continue;
            };
            if !is_enrolled(name) {
                // Functions outside a `@require_contracts` module keep
                // today's lax behaviour — provenance tags don't enrol
                // (RES-3858).
                continue;
            }
            let Some(info) = contracts.get(name) else {
                continue;
            };
            // Strict policy: contracts must be *present*, not merely
            // non-vacuous when declared. `requires` is only demanded
            // of functions with parameters (a parameterless function
            // has no input domain to constrain), and `main` is exempt
            // as the entry point.
            if strict && name != "main" {
                if info.requires.is_empty() && !info.params.is_empty() {
                    errors.push(diagnostic(
                        source_path,
                        info.line,
                        name,
                        "strict policy demands at least one `requires` clause \
                         constraining its inputs — add `requires <param_condition>`",
                    ));
                }
                if info.ensures.is_empty() {
                    errors.push(diagnostic(
                        source_path,
                        info.line,
                        name,
                        "strict policy demands at least one `ensures` clause \
                         constraining its output — add `ensures result <condition>`",
                    ));
                }
            }
            // Bare `@require_contracts` judges declared clauses only;
            // iterating zero clauses is naturally a no-op.
            for msg in contract_clause_errors(info) {
                errors.push(diagnostic(source_path, info.line, name, &msg));
            }
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
    fn directive_parses_and_registers_module_key() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src =
            "@require_contracts\nfn f(int x) requires x >= 0 ensures result >= 0 { return x; }";
        let (_prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        assert!(module_requires_contracts());
        let entries = crate::feature_attrs::find_kind("require_contracts");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, MODULE_KEY);
        crate::feature_attrs::reset();
    }

    #[test]
    fn enrolment_via_module_directive_covers_all_functions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            MODULE_KEY,
            crate::feature_attrs::AttrRecord {
                name: "require_contracts".into(),
                args: String::new(),
                line: 1,
            },
        );
        assert!(is_enrolled("any_function_at_all"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn ai_generated_tag_does_not_enrol() {
        // RES-3858: provenance grants no verification powers.
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "tagged",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: String::new(),
                line: 1,
            },
        );
        assert!(!is_enrolled("tagged"));
        assert!(!is_enrolled("untagged"));
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_vacuous_requires_on_untagged_function() {
        let err = parse_and_check(
            "@require_contracts\nfn f(int x) requires true ensures result >= 0 { return x; }",
        )
        .expect_err("vacuous requires must be rejected under @require_contracts");
        assert!(err.contains("error[contract_policy]"), "unexpected: {err}");
        assert!(err.contains("vacuous precondition"), "unexpected: {err}");
    }

    #[test]
    fn rejects_ensures_not_referencing_result() {
        let err = parse_and_check(
            "@require_contracts\nfn f(int x) requires x >= 0 ensures x >= 0 { return x; }",
        )
        .expect_err("ensures without result must be rejected");
        assert!(
            err.contains("does not reference `result`"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn accepts_real_contracts_on_untagged_function() {
        assert!(
            parse_and_check(
                "@require_contracts\nfn f(int x) requires x >= 0 ensures result >= x { return x + 1; }"
            )
            .is_ok()
        );
    }

    #[test]
    fn contractless_functions_pass_under_bare_directive() {
        assert!(parse_and_check("@require_contracts\nfn f(int x) { return x + 1; }").is_ok());
    }

    #[test]
    fn no_directive_means_no_enforcement() {
        // Same vacuous contract, no directive: today's lax behaviour.
        assert!(parse_and_check("fn f(int x) requires true { return x; }").is_ok());
    }

    #[test]
    fn removing_ai_generated_changes_no_diagnostic() {
        // RES-3858 acceptance: the tag is pure metadata — the check
        // result is identical with and without it, both under the
        // directive and without it.
        let run = |src: &str| {
            let _g = crate::feature_attrs::lock_for_test();
            crate::feature_attrs::reset();
            let (prog, errs) = crate::parse(src);
            assert!(errs.is_empty(), "parse errors: {errs:?}");
            let result = check(&prog, "<test>");
            crate::feature_attrs::reset();
            result
        };
        let body = "fn f(int x) requires true ensures result >= 0 { return x; }";

        // Under the directive: both forms rejected with the same error.
        let tagged = run(&format!("@require_contracts\n@ai_generated\n{body}"));
        let untagged = run(&format!("@require_contracts\n{body}"));
        let tagged_err = tagged.expect_err("vacuous clause rejected");
        let untagged_err = untagged.expect_err("vacuous clause rejected");
        // Only the source line differs (the tag occupies a line).
        assert_eq!(
            tagged_err.replace(":3:", ":N:").replace(":2:", ":N:"),
            untagged_err.replace(":3:", ":N:").replace(":2:", ":N:"),
        );

        // Without the directive: both forms pass — the tag alone
        // triggers nothing.
        assert!(run(&format!("@ai_generated\n{body}")).is_ok());
        assert!(run(body).is_ok());
    }

    #[test]
    fn strict_directive_parses_and_sets_strict_mode() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (_prog, errs) = crate::parse(
            "@require_contracts(strict)\nfn f(int x) requires x >= 0 ensures result >= 0 { return x; }",
        );
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        assert!(module_requires_contracts());
        assert!(strict_mode());
        crate::feature_attrs::reset();
    }

    #[test]
    fn bare_directive_is_not_strict() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (_prog, errs) = crate::parse("@require_contracts\nfn f(int x) { return x; }");
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        assert!(!strict_mode());
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_policy_argument_is_a_parse_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (_prog, errs) = crate::parse("@require_contracts(lenient)\nfn f(int x) { return x; }");
        assert!(
            errs.iter()
                .any(|e| e.contains("unknown @require_contracts policy `lenient`")),
            "expected unknown-policy error, got: {errs:?}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn strict_rejects_contractless_function() {
        let err = parse_and_check("@require_contracts(strict)\nfn f(int x) { return x; }")
            .expect_err("strict mode must demand contracts");
        assert!(err.contains("error[contract_policy]"), "unexpected: {err}");
        assert!(
            err.contains("`requires` clause") && err.contains("`ensures` clause"),
            "expected both presence errors: {err}"
        );
    }

    #[test]
    fn strict_rejects_missing_ensures_only() {
        let err = parse_and_check(
            "@require_contracts(strict)\nfn f(int x) requires x >= 0 { return x; }",
        )
        .expect_err("strict mode must demand ensures");
        assert!(
            err.contains("`ensures` clause") && !err.contains("`requires` clause constraining"),
            "expected only the ensures presence error: {err}"
        );
    }

    #[test]
    fn strict_exempts_main_and_parameterless_requires() {
        // `main` needs no contracts; a parameterless function needs no
        // `requires` (there is no input domain), but still needs `ensures`.
        assert!(
            parse_and_check(
                "@require_contracts(strict)\nfn answer() ensures result == 42 { return 42; }\nfn main() { println(answer()); }"
            )
            .is_ok()
        );
    }

    #[test]
    fn strict_parameterless_function_still_needs_ensures() {
        let err = parse_and_check("@require_contracts(strict)\nfn answer() { return 42; }")
            .expect_err("strict mode must demand ensures on parameterless functions");
        assert!(err.contains("`ensures` clause"), "unexpected: {err}");
    }

    #[test]
    fn strict_still_applies_vacuity_rules() {
        let err = parse_and_check(
            "@require_contracts(strict)\nfn f(int x) requires true ensures result >= 0 { return x; }",
        )
        .expect_err("strict mode keeps the vacuity rules");
        assert!(err.contains("vacuous precondition"), "unexpected: {err}");
    }

    #[test]
    fn typechecker_rejects_vacuous_contract_under_directive() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "@require_contracts\nfn f(int x) requires true ensures result >= 0 { return x; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let mut tc = crate::typechecker::TypeChecker::new();
        let err = tc
            .check_program_with_source(&prog, "<test>")
            .expect_err("typecheck must reject vacuous contract under @require_contracts");
        assert!(
            err.contains("error[contract_policy]"),
            "expected contract_policy diagnostic, got: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn standalone_directive_at_end_of_file_parses() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (_prog, errs) = crate::parse("fn f(int x) { return x; }\n@require_contracts");
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        assert!(module_requires_contracts());
        crate::feature_attrs::reset();
    }
}
