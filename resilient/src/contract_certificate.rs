//! RES-3859 (#3854 Tier 3): contract proof certificates.
//!
//! `rz <file> --emit-contract-certificate <path>` writes a portable,
//! offline JSON document attesting — per function, per clause — what
//! the Z3 verifier actually established: `pass` (with the replayable
//! SMT-LIB2 certificate when the prover produced one), `fail` (with a
//! counterexample when Z3 gave a model), or `unknown` (out of the
//! supported subset, solver timeout, or a build without `--features
//! z3`; runtime checks remain).
//!
//! The certificate attests the *proof*, not the provenance. Whether a
//! function is tagged `@ai_generated` / `#[generated]` appears only as
//! an informational `provenance` array and changes nothing else in the
//! document — verification is a property of the code, not of who wrote
//! it (#3854).
//!
//! Output is deterministic: functions in program order, clauses in
//! `contract_verify::verify_program` order (declared `requires`, then
//! declared `ensures`, then inferred clauses), and every JSON object
//! writes its keys in a fixed order. Suitable for golden tests and
//! byte-for-byte audit diffs.

use std::fmt::Write as _;

use crate::Node;
use crate::contract_verify::{ClauseKind, ClauseVerdict, Verdict};

const SCHEMA: &str = "resilient-contract-certificate/v1";

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn kind_label(kind: ClauseKind) -> &'static str {
    match kind {
        ClauseKind::Requires => "requires",
        ClauseKind::Ensures => "ensures",
        ClauseKind::InferredRequires => "inferred_requires",
        ClauseKind::InferredEnsures => "inferred_ensures",
    }
}

fn write_clause(out: &mut String, v: &ClauseVerdict) {
    out.push_str("      {\n");
    let _ = writeln!(out, "        \"clause\": \"{}\",", json_escape(&v.clause));
    let _ = writeln!(out, "        \"kind\": \"{}\",", kind_label(v.kind));
    // RES-3969: `basis` records whether an `ensures` verdict was
    // proven against the substituted function body (`implementation`)
    // or the free-variable clause text (`clause-only`). Emitted only
    // for `ensures`-family clauses — it is meaningless for `requires`
    // and consistency checks, which never substitute `result`.
    if matches!(v.kind, ClauseKind::Ensures | ClauseKind::InferredEnsures) {
        let _ = writeln!(out, "        \"basis\": \"{}\",", v.basis.label());
    }
    match &v.verdict {
        Verdict::Pass { certificate } => {
            if let Some(cert) = certificate {
                let _ = writeln!(out, "        \"smtlib2\": \"{}\",", json_escape(cert));
            }
            out.push_str("        \"verdict\": \"pass\"\n");
        }
        Verdict::Fail { counterexample } => {
            if let Some(cx) = counterexample {
                let _ = writeln!(out, "        \"counterexample\": \"{}\",", json_escape(cx));
            }
            out.push_str("        \"verdict\": \"fail\"\n");
        }
        Verdict::Unknown => {
            out.push_str("        \"verdict\": \"unknown\"\n");
        }
    }
    out.push_str("      }");
}

/// Provenance tags recorded for `item` — informational only.
fn provenance_tags(item: &str) -> Vec<&'static str> {
    let mut tags = Vec::new();
    if crate::feature_attrs::find_kind("ai_generated")
        .iter()
        .any(|(name, _)| name == item)
    {
        tags.push("ai_generated");
    }
    if crate::feature_attrs::find_kind("generated")
        .iter()
        .any(|(name, _)| name == item)
    {
        tags.push("generated");
    }
    tags
}

/// Build the certificate JSON document for `program`.
///
/// Every top-level function appears, in program order, with its
/// enrolment status ([`crate::contract_policy::is_enrolled`]), its
/// informational provenance tags, and one entry per contract clause
/// routed through the prover.
pub(crate) fn emit(program: &Node, source_path: &str) -> String {
    let verdicts = crate::contract_verify::verify_program(program);

    let mut out = String::new();
    out.push_str("{\n");
    let _ = writeln!(out, "  \"schema\": \"{}\",", json_escape(SCHEMA));
    let _ = writeln!(out, "  \"source\": \"{}\",", json_escape(source_path));
    out.push_str("  \"functions\": [\n");

    let mut first_fn = true;
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            let Node::Function { name, .. } = &stmt.node else {
                continue;
            };
            if !first_fn {
                out.push_str(",\n");
            }
            first_fn = false;

            out.push_str("    {\n");
            let _ = writeln!(out, "      \"name\": \"{}\",", json_escape(name));
            let _ = writeln!(
                out,
                "      \"enrolled\": {},",
                crate::contract_policy::is_enrolled(name)
            );
            let tags = provenance_tags(name);
            let tags_json = tags
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "      \"provenance\": [{tags_json}],");
            out.push_str("      \"clauses\": [\n");
            let mut first_clause = true;
            for v in verdicts.iter().filter(|v| &v.function_name == name) {
                if !first_clause {
                    out.push_str(",\n");
                }
                first_clause = false;
                write_clause(&mut out, v);
            }
            if !first_clause {
                out.push('\n');
            }
            out.push_str("      ]\n    }");
        }
    }
    if !first_fn {
        out.push('\n');
    }
    out.push_str("  ]\n}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn emit_for(src: &str) -> String {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let json = emit(&prog, "<test>");
        crate::feature_attrs::reset();
        json
    }

    #[test]
    fn certificate_is_valid_shaped_json_with_fixed_keys() {
        let json = emit_for(
            "fn div(int a, int b) requires b != 0 ensures result == a / b { return a / b; }",
        );
        assert!(json.starts_with("{\n  \"schema\": \"resilient-contract-certificate/v1\",\n"));
        assert!(json.contains("\"source\": \"<test>\""));
        assert!(json.contains("\"name\": \"div\""));
        assert!(json.contains("\"kind\": \"requires\""));
        assert!(json.contains("\"kind\": \"ensures\""));
        assert!(
            json.contains("\"clause\": \"(b != 0)\"") || json.contains("\"clause\": \"b != 0\"")
        );
        assert!(json.ends_with("}\n"));
    }

    #[test]
    fn certificate_is_deterministic() {
        let src = "fn f(int x) requires x >= 0 ensures result >= 0 { return x; }\nfn g(int y) requires y > 0 { return y; }";
        assert_eq!(emit_for(src), emit_for(src));
    }

    #[test]
    fn provenance_is_informational_only() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) requires x >= 0 ensures result >= 0 { return x; }";
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        let untagged = emit(&prog, "<test>");

        crate::feature_attrs::record(
            "f",
            crate::feature_attrs::AttrRecord {
                name: "ai_generated".into(),
                args: String::new(),
                line: 1,
            },
        );
        let tagged = emit(&prog, "<test>");
        crate::feature_attrs::reset();

        assert!(untagged.contains("\"provenance\": []"));
        assert!(tagged.contains("\"provenance\": [\"ai_generated\"]"));
        // The tag must change nothing beyond the informational fields:
        // strip the provenance and enrolment lines and the documents
        // are identical.
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.contains("\"provenance\"") && !l.contains("\"enrolled\""))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(strip(&untagged), strip(&tagged));
    }

    #[test]
    fn functions_without_contracts_have_empty_clauses() {
        let json = emit_for("fn plain(int x) { return x; }");
        assert!(json.contains("\"name\": \"plain\""));
        assert!(json.contains("\"enrolled\": false"));
    }

    #[cfg(not(feature = "z3"))]
    #[test]
    fn non_z3_build_reports_unknown_golden() {
        let json = emit_for("fn f(int x) requires x >= 0 ensures result >= 0 { return x; }");
        let expected = "{\n  \"schema\": \"resilient-contract-certificate/v1\",\n  \"source\": \"<test>\",\n  \"functions\": [\n    {\n      \"name\": \"f\",\n      \"enrolled\": false,\n      \"provenance\": [],\n      \"clauses\": [\n";
        assert!(json.starts_with(expected), "unexpected prefix:\n{json}");
        // Without z3 every clause is Unknown and carries no SMT dump.
        assert!(json.contains("\"verdict\": \"unknown\""));
        assert!(!json.contains("\"verdict\": \"pass\""));
        assert!(!json.contains("\"smtlib2\""));
    }

    #[cfg(feature = "z3")]
    #[test]
    fn z3_build_attests_discharged_tautology() {
        // `x >= 0 || x < 0` is a tautology — Z3 discharges it and the
        // certificate must attest the proof with a replayable dump.
        let json =
            emit_for("fn f(int x) requires x >= 0 || x < 0 ensures result == x { return x; }");
        assert!(
            json.contains("\"verdict\": \"pass\""),
            "expected a pass verdict:\n{json}"
        );
        assert!(
            json.contains("\"smtlib2\": \""),
            "pass verdict should carry the SMT-LIB2 certificate:\n{json}"
        );
    }
}
