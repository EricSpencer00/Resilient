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
//! ## Division of labour with `ai_generated.rs`
//!
//! Functions tagged `@ai_generated` are skipped here: that pass owns
//! them and additionally enforces clause *presence* (both `requires`
//! and `ensures` must exist). Under the bare `@require_contracts`
//! directive, functions with no contract clauses at all are accepted —
//! mandatory presence is the opt-in *strict* policy (follow-up PR in
//! the #3854 chain).

use std::collections::HashSet;

use crate::Node;

/// Registry key the parser records the module-level directive under.
/// The directive attaches to the file, not to any item, so it uses a
/// reserved pseudo-item name no real function can shadow.
pub(crate) const MODULE_KEY: &str = "<module>";

/// True when the current file declared `@require_contracts`.
pub(crate) fn module_requires_contracts() -> bool {
    !crate::feature_attrs::find_kind("require_contracts").is_empty()
}

/// RES-3854 enrolment predicate: is `fn_name` subject to contract
/// verification? True under the module-level `@require_contracts`
/// directive (every function is enrolled) or when the function carries
/// the `@ai_generated` provenance tag. Downstream verification passes
/// (Tier 2 loop bounds, proof certificates) share this single source
/// of truth instead of re-deriving enrolment from provenance.
pub(crate) fn is_enrolled(fn_name: &str) -> bool {
    if module_requires_contracts() {
        return true;
    }
    crate::feature_attrs::find_kind("ai_generated")
        .iter()
        .any(|(item, _)| item == fn_name)
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
    let ai_tagged: HashSet<String> = crate::feature_attrs::find_kind("ai_generated")
        .into_iter()
        .map(|(item, _)| item)
        .collect();

    let contracts = crate::ai_generated::collect_contracts(program);
    let mut errors: Vec<String> = Vec::new();

    // Iterate the program (not the contract map) so diagnostics come
    // out in source order deterministically.
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            let Node::Function { name, .. } = &stmt.node else {
                continue;
            };
            if !is_enrolled(name) || ai_tagged.contains(name) {
                // Untagged functions outside a `@require_contracts`
                // module keep today's lax behaviour; `@ai_generated`
                // functions are owned by that stricter pass.
                continue;
            }
            let Some(info) = contracts.get(name) else {
                continue;
            };
            if info.requires.is_empty() && info.ensures.is_empty() {
                // Bare `@require_contracts` judges declared clauses
                // only; mandatory presence is strict mode.
                continue;
            }
            for msg in crate::ai_generated::contract_clause_errors(info) {
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
    fn enrolment_via_ai_generated_tag_is_per_function() {
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
        assert!(is_enrolled("tagged"));
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
    fn ai_generated_functions_are_delegated_to_that_pass() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "@require_contracts\n@ai_generated\nfn f(int x) requires true ensures result >= 0 { return x; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        // contract_policy skips the tagged function...
        assert!(check(&prog, "<test>").is_ok());
        // ...because ai_generated::check owns it and still rejects it.
        let err = crate::ai_generated::check(&prog, "<test>")
            .expect_err("ai_generated pass still rejects vacuous clause");
        assert!(err.contains("error[ai_generated]"), "unexpected: {err}");
        crate::feature_attrs::reset();
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
