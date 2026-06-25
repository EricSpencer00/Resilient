//! `@ai_generated` validation.
//!
//! AI-generated functions are allowed only when they carry explicit
//! `requires` or `ensures` clauses. The verifier can then reason about
//! the stated contract without making any network call or trusting an
//! external model.

use std::collections::HashMap;

use crate::Node;

fn diagnostic(source_path: &str, line: usize, fn_name: &str, message: &str) -> String {
    format!(
        "{source_path}:{line}:0: error[ai_generated]: invalid @ai_generated declaration `{fn_name}`: {message}"
    )
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("ai_generated");
    if attrs.is_empty() {
        return Ok(());
    }

    let mut contract_counts = HashMap::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                requires,
                ensures,
                ..
            } = &stmt.node
            {
                contract_counts.insert(name.clone(), requires.len() + ensures.len());
            }
        }
    }

    for (fn_name, rec) in attrs {
        let Some(contract_count) = contract_counts.get(&fn_name) else {
            continue;
        };
        if *contract_count == 0 {
            return Err(diagnostic(
                source_path,
                rec.line,
                &fn_name,
                "function must declare at least one `requires` or `ensures` clause",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_at_ai_generated_attribute() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "@ai_generated\nfn made_by_model(int x) requires x >= 0 { return x + 1; }";
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
        assert_eq!(
            err,
            "<test>:1:0: error[ai_generated]: invalid @ai_generated declaration `made_by_model`: function must declare at least one `requires` or `ensures` clause"
        );
        crate::feature_attrs::reset();
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
            err.contains("error[ai_generated]")
                && err
                    .contains("function must declare at least one `requires` or `ensures` clause"),
            "expected ai_generated missing-contract diagnostic, got: {err}"
        );
        crate::feature_attrs::reset();
    }
}
