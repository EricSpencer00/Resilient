//! RES-3779 Phase A — unified contract verification pipeline.
//!
//! Wires the contract surface into the Z3 proving path and reports a
//! stable pass/fail/unknown verdict per clause:
//!
//! * **Declared `requires`** clauses are checked for internal
//!   consistency. A clause that is a contradiction (`x > 0 && x < 0`)
//!   can never be satisfied by any caller — that is a `Fail`. A
//!   tautology is a `Pass` with an SMT-LIB2 proof certificate.
//! * **Declared `ensures`** clauses are discharged under the fn's
//!   `requires` clauses as axioms. A clause proven to hold for every
//!   input admitted by the preconditions is a `Pass` (with
//!   certificate); a clause that contradicts the preconditions is a
//!   `Fail` (with counterexample when Z3 produces a model).
//! * **Inferred contracts** (from [`crate::contract_inference`]) in
//!   the machine-checkable subset (`p != 0`, `p > 0`) are routed
//!   through the same path, so suggested contracts arrive
//!   pre-vetted.
//!
//! Any clause outside the Z3 translator's supported subset — or every
//! clause when the crate is built without `--features z3` — reports
//! `Unknown`: the runtime check is retained and nothing is claimed
//! statically. Verdicts are advisory diagnostics; they never reject a
//! program.
//!
//! Remaining RES-3779 phases (TLA+ model checking, fuzz routing) land
//! in follow-up PRs.

#![allow(dead_code)]

use crate::Node;

/// Which contract surface a verdict refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClauseKind {
    Requires,
    Ensures,
    InferredRequires,
    InferredEnsures,
}

impl ClauseKind {
    fn label(self) -> &'static str {
        match self {
            ClauseKind::Requires => "requires",
            ClauseKind::Ensures => "ensures",
            ClauseKind::InferredRequires => "inferred requires",
            ClauseKind::InferredEnsures => "inferred ensures",
        }
    }
}

/// Outcome of routing one clause through the prover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Statically proven. `certificate` holds a self-contained
    /// SMT-LIB2 dump that stock Z3 replays to `unsat` (the negated
    /// clause is unsatisfiable), when the prover produced one.
    Pass { certificate: Option<String> },
    /// Statically refuted — the clause cannot hold. `counterexample`
    /// holds a falsifying assignment (`a = -1, b = 0`) when Z3
    /// produced a model.
    Fail { counterexample: Option<String> },
    /// Undecided: out of the supported subset, solver timeout, or the
    /// crate was built without `--features z3`. Runtime checks remain.
    Unknown,
}

/// One clause's verdict, addressable enough for stable diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClauseVerdict {
    pub function_name: String,
    pub kind: ClauseKind,
    /// Source-ish rendering of the clause (`b != 0`).
    pub clause: String,
    pub verdict: Verdict,
}

/// Route every contract clause in `program` through the prover.
///
/// Order is deterministic: functions in program order; per function,
/// declared `requires`, then declared `ensures`, then inferred
/// clauses in inference order.
pub fn verify_program(program: &Node) -> Vec<ClauseVerdict> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    let inferred = crate::contract_inference::infer_program(program);
    let mut out = Vec::new();
    for s in stmts {
        let Node::Function {
            name,
            requires,
            ensures,
            ..
        } = &s.node
        else {
            continue;
        };
        for r in requires {
            out.push(ClauseVerdict {
                function_name: name.clone(),
                kind: ClauseKind::Requires,
                clause: render_expr(r),
                verdict: prove_clause(r, &[]),
            });
        }
        for e in ensures {
            out.push(ClauseVerdict {
                function_name: name.clone(),
                kind: ClauseKind::Ensures,
                clause: render_expr(e),
                verdict: prove_clause(e, requires),
            });
        }
        if let Some(inf) = inferred.iter().find(|i| &i.function_name == name) {
            for (kind, clauses) in [
                (ClauseKind::InferredRequires, &inf.requires),
                (ClauseKind::InferredEnsures, &inf.ensures),
            ] {
                for c in clauses {
                    let verdict = match parse_inferred_clause(c) {
                        Some(expr) => prove_clause(&expr, &[]),
                        None => Verdict::Unknown,
                    };
                    out.push(ClauseVerdict {
                        function_name: name.clone(),
                        kind,
                        clause: c.clone(),
                        verdict,
                    });
                }
            }
        }
    }
    out
}

/// Human-readable report, one line per clause:
///
/// ```text
/// PASS     div: requires `b != 0 || b == 0` [certified]
/// FAIL     f: requires `x > 0 && x < 0` (counterexample: x = 0)
/// UNKNOWN  g: ensures `result >= 0`
/// ```
pub fn format_report(verdicts: &[ClauseVerdict]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    for v in verdicts {
        let _ = match &v.verdict {
            Verdict::Pass { certificate } => writeln!(
                s,
                "PASS     {}: {} `{}`{}",
                v.function_name,
                v.kind.label(),
                v.clause,
                if certificate.is_some() {
                    " [certified]"
                } else {
                    ""
                }
            ),
            Verdict::Fail { counterexample } => match counterexample {
                Some(cx) => writeln!(
                    s,
                    "FAIL     {}: {} `{}` (counterexample: {})",
                    v.function_name,
                    v.kind.label(),
                    v.clause,
                    cx
                ),
                None => writeln!(
                    s,
                    "FAIL     {}: {} `{}`",
                    v.function_name,
                    v.kind.label(),
                    v.clause
                ),
            },
            Verdict::Unknown => writeln!(
                s,
                "UNKNOWN  {}: {} `{}`",
                v.function_name,
                v.kind.label(),
                v.clause
            ),
        };
    }
    s
}

/// Per-clause Z3 timeout. Contract clauses are small (a handful of
/// LIA atoms), so anything Z3 can decide it decides in well under a
/// second; the cap only bounds pathological inputs.
#[cfg(feature = "z3")]
const CLAUSE_TIMEOUT_MS: u32 = 1000;

#[cfg(feature = "z3")]
fn prove_clause(expr: &Node, axioms: &[Node]) -> Verdict {
    let (verdict, cert, cx, _timed_out) = crate::verifier_z3::prove_with_axioms_and_timeout(
        expr,
        &std::collections::HashMap::new(),
        axioms,
        CLAUSE_TIMEOUT_MS,
    );
    match verdict {
        Some(true) => Verdict::Pass {
            certificate: cert.map(|c| c.smt2),
        },
        Some(false) => Verdict::Fail { counterexample: cx },
        None => Verdict::Unknown,
    }
}

#[cfg(not(feature = "z3"))]
fn prove_clause(_expr: &Node, _axioms: &[Node]) -> Verdict {
    Verdict::Unknown
}

/// Parse the machine-checkable subset of inferred-contract strings
/// back into AST expressions: `IDENT != 0` and `IDENT > 0`. The other
/// inference shapes (`len(p) > 0`, `p != null`, `result == <expr>`)
/// involve theories the LIA path doesn't model faithfully for this
/// purpose, so they stay `Unknown` rather than risking a vacuous
/// verdict.
fn parse_inferred_clause(clause: &str) -> Option<Node> {
    let mut parts = clause.split_whitespace();
    let (ident, op, rhs) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || rhs != "0" {
        return None;
    }
    let operator = match op {
        "!=" => "!=",
        ">" => ">",
        _ => return None,
    };
    if ident.is_empty() || !ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    let sp = crate::span::Span::default();
    Some(Node::InfixExpression {
        left: Box::new(Node::Identifier {
            name: ident.to_string(),
            span: sp,
        }),
        operator,
        right: Box::new(Node::IntegerLiteral { value: 0, span: sp }),
        span: sp,
    })
}

/// Render a clause expression for diagnostics. Falls back to the
/// Debug dump outside the common contract-expression shapes — the
/// verdict machinery doesn't depend on this string.
fn render_expr(expr: &Node) -> String {
    match expr {
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::PrefixExpression {
            operator, right, ..
        } => format!("{operator}{}", render_expr(right)),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!("{} {} {}", render_expr(left), operator, render_expr(right)),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let args: Vec<String> = arguments.iter().map(render_expr).collect();
            format!("{}({})", render_expr(function), args.join(", "))
        }
        _ => format!("{expr:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn verdict_for<'a>(
        verdicts: &'a [ClauseVerdict],
        func: &str,
        kind: ClauseKind,
    ) -> &'a ClauseVerdict {
        verdicts
            .iter()
            .find(|v| v.function_name == func && v.kind == kind)
            .unwrap_or_else(|| panic!("no {kind:?} verdict for {func}"))
    }

    #[test]
    fn empty_program_yields_no_verdicts() {
        let (prog, _) = parse("");
        assert!(verify_program(&prog).is_empty());
    }

    #[test]
    fn declared_clauses_are_enumerated_in_order() {
        let src = r#"
            fn div(int a, int b) -> int
                requires b != 0
                ensures result == a / b
            { return a / b; }
        "#;
        let (prog, _) = parse(src);
        let verdicts = verify_program(&prog);
        assert_eq!(verdicts.len(), 2);
        assert_eq!(verdicts[0].kind, ClauseKind::Requires);
        assert_eq!(verdicts[0].clause, "b != 0");
        assert_eq!(verdicts[1].kind, ClauseKind::Ensures);
        assert_eq!(verdicts[1].clause, "result == a / b");
    }

    #[test]
    fn inferred_clauses_are_routed() {
        // No declared contracts; inference proposes `requires b != 0`
        // for the division, which is in the checkable subset.
        let src = "fn divide(int a, int b) -> int { return a / b; }";
        let (prog, _) = parse(src);
        let verdicts = verify_program(&prog);
        let v = verdict_for(&verdicts, "divide", ClauseKind::InferredRequires);
        assert_eq!(v.clause, "b != 0");
    }

    #[test]
    fn parse_inferred_clause_supported_subset() {
        assert!(parse_inferred_clause("b != 0").is_some());
        assert!(parse_inferred_clause("n > 0").is_some());
        // Out-of-subset shapes stay unparsed → Unknown, never a
        // vacuous verdict.
        assert!(parse_inferred_clause("len(p) > 0").is_none());
        assert!(parse_inferred_clause("p != null").is_none());
        assert!(parse_inferred_clause("result == a / b").is_none());
        assert!(parse_inferred_clause("b != 1").is_none());
    }

    #[test]
    fn format_report_one_line_per_clause() {
        let verdicts = vec![
            ClauseVerdict {
                function_name: "f".into(),
                kind: ClauseKind::Requires,
                clause: "x > 0".into(),
                verdict: Verdict::Pass {
                    certificate: Some("(check-sat)".into()),
                },
            },
            ClauseVerdict {
                function_name: "f".into(),
                kind: ClauseKind::Ensures,
                clause: "result >= 0".into(),
                verdict: Verdict::Fail {
                    counterexample: Some("x = 1".into()),
                },
            },
            ClauseVerdict {
                function_name: "g".into(),
                kind: ClauseKind::InferredRequires,
                clause: "b != 0".into(),
                verdict: Verdict::Unknown,
            },
        ];
        let report = format_report(&verdicts);
        let lines: Vec<&str> = report.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "PASS     f: requires `x > 0` [certified]");
        assert_eq!(
            lines[1],
            "FAIL     f: ensures `result >= 0` (counterexample: x = 1)"
        );
        assert_eq!(lines[2], "UNKNOWN  g: inferred requires `b != 0`");
    }

    #[cfg(not(feature = "z3"))]
    #[test]
    fn without_z3_every_verdict_is_unknown() {
        let src = r#"
            fn f(int x) -> int
                requires x > 0
                ensures result >= 0
            { return x; }
        "#;
        let (prog, _) = parse(src);
        for v in verify_program(&prog) {
            assert_eq!(v.verdict, Verdict::Unknown, "clause `{}`", v.clause);
        }
    }

    #[cfg(feature = "z3")]
    mod z3 {
        use super::*;

        #[test]
        fn tautological_requires_passes_with_certificate() {
            let src = r#"
                fn f(int x) -> int
                    requires x > 0 || x <= 0
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "f", ClauseKind::Requires);
            match &v.verdict {
                Verdict::Pass { certificate } => {
                    let cert = certificate.as_ref().expect("certificate emitted");
                    assert!(cert.contains("check-sat"), "not SMT-LIB2: {cert}");
                }
                other => panic!("expected Pass, got {other:?}"),
            }
        }

        #[test]
        fn contradictory_requires_fails() {
            let src = r#"
                fn f(int x) -> int
                    requires x > 0 && x < 0
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "f", ClauseKind::Requires);
            assert!(
                matches!(v.verdict, Verdict::Fail { .. }),
                "expected Fail, got {:?}",
                v.verdict
            );
        }

        #[test]
        fn ensures_discharged_under_requires_axioms() {
            // `x >= 0` is not a tautology, but it follows from the
            // precondition `x > 0`.
            let src = r#"
                fn f(int x) -> int
                    requires x > 0
                    ensures x >= 0
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "f", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Pass { .. }),
                "expected Pass, got {:?}",
                v.verdict
            );
        }

        #[test]
        fn satisfiable_non_tautology_is_unknown() {
            // `x > 0` alone: some inputs satisfy it, some don't —
            // not statically decided, runtime check retained.
            let src = r#"
                fn f(int x) -> int
                    requires x > 0
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "f", ClauseKind::Requires);
            assert_eq!(v.verdict, Verdict::Unknown);
        }

        #[test]
        fn inferred_divide_by_zero_guard_reports_unknown_not_pass() {
            // `b != 0` is satisfiable but not a tautology — the
            // pipeline must not stamp a suggested contract as proven.
            let src = "fn divide(int a, int b) -> int { return a / b; }";
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "divide", ClauseKind::InferredRequires);
            assert_eq!(v.verdict, Verdict::Unknown);
        }
    }
}
