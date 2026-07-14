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

mod symbolic_eval;

use crate::Node;
use symbolic_eval::ResultModel;

/// Whether an `ensures` verdict was established against the function's
/// actual implementation or only against the clause text.
///
/// RES-3969: the free-variable `ensures` proof leaves `result`
/// unconstrained, so a clause like `result >= x` proves for *some*
/// `result` regardless of what the body returns — a wrong `max` that
/// returns `x` verifies identically to a correct one. `Implementation`
/// marks a verdict where the body's return expression was substituted
/// for `result` (via [`symbolic_eval`]), so the obligation is grounded
/// in what the function actually computes. `ClauseOnly` marks the
/// free-variable fallback used for `requires`/inferred clauses and for
/// `ensures` on bodies outside the modelled subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofBasis {
    Implementation,
    ClauseOnly,
}

impl ProofBasis {
    pub fn label(self) -> &'static str {
        match self {
            ProofBasis::Implementation => "implementation",
            ProofBasis::ClauseOnly => "clause-only",
        }
    }
}

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
    /// RES-3969: for `ensures` clauses, whether the verdict was proven
    /// against the substituted function body (`Implementation`) or the
    /// free-variable clause text (`ClauseOnly`). Always `ClauseOnly`
    /// for `requires` and inferred clauses.
    pub basis: ProofBasis,
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
            body,
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
                basis: ProofBasis::ClauseOnly,
            });
        }
        for e in ensures {
            let (verdict, basis) = prove_ensures(e, requires, body);
            out.push(ClauseVerdict {
                function_name: name.clone(),
                kind: ClauseKind::Ensures,
                clause: render_expr(e),
                verdict,
                basis,
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
                        basis: ProofBasis::ClauseOnly,
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

/// RES-3969: prove an `ensures` clause against the function body.
///
/// When the clause constrains `result` and the body is in the
/// [`symbolic_eval`] subset, the body's return expression is
/// substituted for `result` and the *grounded* obligation is proven —
/// `Implementation` basis. For a branching body, each branch is proven
/// under its path condition (asserted through the axiom channel) and
/// the results combined: every reachable branch must discharge the
/// clause. Otherwise the free-variable clause text is proven directly —
/// `ClauseOnly` basis — exactly the pre-RES-3969 behaviour, retained as
/// an explicit, labeled fallback for out-of-subset bodies.
fn prove_ensures(clause: &Node, requires: &[Node], body: &Node) -> (Verdict, ProofBasis) {
    if symbolic_eval::mentions_result(clause)
        && let Some(model) = symbolic_eval::model_body(body)
    {
        let verdict = match model {
            ResultModel::Straight { ret } => {
                let obligation = symbolic_eval::substitute_result(clause, &ret);
                prove_obligation(&obligation, requires)
            }
            ResultModel::Branch {
                condition,
                then_ret,
                else_ret,
            } => {
                let then_obligation = symbolic_eval::substitute_result(clause, &then_ret);
                let mut then_axioms = requires.to_vec();
                then_axioms.push((*condition).clone());
                let then_verdict = prove_obligation(&then_obligation, &then_axioms);

                let else_obligation = symbolic_eval::substitute_result(clause, &else_ret);
                let mut else_axioms = requires.to_vec();
                else_axioms.push(symbolic_eval::negate(&condition));
                let else_verdict = prove_obligation(&else_obligation, &else_axioms);

                combine_branch_verdicts(then_verdict, else_verdict)
            }
        };
        return (verdict, ProofBasis::Implementation);
    }
    (prove_clause(clause, requires), ProofBasis::ClauseOnly)
}

/// Combine the two per-branch verdicts of an `if/else` case split: the
/// clause holds for the whole function only if it holds on *both*
/// reachable branches. A refutation on either branch refutes the
/// clause; anything short of two proofs is `Unknown`.
fn combine_branch_verdicts(then_v: Verdict, else_v: Verdict) -> Verdict {
    match (then_v, else_v) {
        (Verdict::Fail { counterexample }, _) | (_, Verdict::Fail { counterexample }) => {
            Verdict::Fail { counterexample }
        }
        (Verdict::Pass { certificate: a }, Verdict::Pass { certificate: b }) => Verdict::Pass {
            certificate: merge_certificates(a, b),
        },
        _ => Verdict::Unknown,
    }
}

fn merge_certificates(a: Option<String>, b: Option<String>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!(
            "; RES-3969 branch case-split\n; then-branch:\n{a}\n; else-branch:\n{b}"
        )),
        (Some(c), None) | (None, Some(c)) => Some(c),
        (None, None) => None,
    }
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

/// RES-3969: prove a *grounded* `ensures` obligation — one where the
/// body's return expression has already been substituted for `result`,
/// so `expr` is a closed formula over the parameters.
///
/// Unlike [`prove_clause`], which only refutes a clause that is a
/// standalone contradiction, this uses **validity** semantics suited to
/// partial-correctness postconditions: the obligation must hold for
/// *every* input admitted by the preconditions. A model of
/// `axioms ∧ ¬obligation` is therefore a concrete input that satisfies
/// the preconditions yet violates the postcondition — a genuine
/// refutation (`Fail` with counterexample), not an "undecided". The
/// solver already extracts exactly that model on the tautology check;
/// we only reach for it when the negation was genuinely satisfiable
/// (`!timed_out` and a counterexample was produced), so an untranslated
/// obligation or a solver timeout still degrades to `Unknown`.
///
/// This validity reading is sound *only because* `result` has been
/// pinned to the body: the caller uses [`prove_clause`] for the
/// free-variable fallback, where an unconstrained `result` must not be
/// reported as refuted.
#[cfg(feature = "z3")]
fn prove_obligation(expr: &Node, axioms: &[Node]) -> Verdict {
    let (verdict, cert, cx, timed_out) = crate::verifier_z3::prove_with_axioms_and_timeout(
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
        None if !timed_out && cx.is_some() => Verdict::Fail { counterexample: cx },
        None => Verdict::Unknown,
    }
}

#[cfg(not(feature = "z3"))]
fn prove_obligation(_expr: &Node, _axioms: &[Node]) -> Verdict {
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
                basis: ProofBasis::ClauseOnly,
            },
            ClauseVerdict {
                function_name: "f".into(),
                kind: ClauseKind::Ensures,
                clause: "result >= 0".into(),
                verdict: Verdict::Fail {
                    counterexample: Some("x = 1".into()),
                },
                basis: ProofBasis::Implementation,
            },
            ClauseVerdict {
                function_name: "g".into(),
                kind: ClauseKind::InferredRequires,
                clause: "b != 0".into(),
                verdict: Verdict::Unknown,
                basis: ProofBasis::ClauseOnly,
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

        // RES-3969: THE regression that proves body-aware `ensures`.
        // A wrong `max` that returns `x` unconditionally must now be
        // REFUTED against `ensures result >= x && result >= y`, because
        // substituting the body gives `x >= x && x >= y`, which fails
        // for `y > x`. Under the pre-RES-3969 free-variable proof this
        // clause was merely satisfiable (`Unknown`) and a wrong `max`
        // was indistinguishable from a correct one.
        #[test]
        fn wrong_max_returning_x_is_refuted() {
            let src = r#"
                fn max(int x, int y) -> int
                    ensures result >= x && result >= y
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "max", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Fail { .. }),
                "wrong max must FAIL its ensures, got {:?}",
                v.verdict
            );
            assert_eq!(v.basis, ProofBasis::Implementation);
        }

        // The correct `if/else` `max` must PASS the same clause: each
        // branch discharges the obligation under its path condition.
        #[test]
        fn correct_max_if_else_passes() {
            let src = r#"
                fn max(int x, int y) -> int
                    ensures result >= x && result >= y
                { if x >= y { return x; } else { return y; } }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "max", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Pass { .. }),
                "correct max must PASS its ensures, got {:?}",
                v.verdict
            );
            assert_eq!(v.basis, ProofBasis::Implementation);
        }

        // The `if C { return T; } return F;` fall-through shape models
        // identically to explicit `if/else`.
        #[test]
        fn correct_max_fallthrough_passes() {
            let src = r#"
                fn max(int x, int y) -> int
                    ensures result >= x && result >= y
                { if x >= y { return x; } return y; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "max", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Pass { .. }),
                "correct fall-through max must PASS, got {:?}",
                v.verdict
            );
        }

        // A straight-line body whose return genuinely satisfies the
        // clause passes on the implementation basis: `identity` returns
        // `x`, and `ensures result == x` substitutes to `x == x`.
        #[test]
        fn straight_line_return_satisfying_clause_passes() {
            let src = r#"
                fn identity(int x) -> int
                    ensures result == x
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "identity", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Pass { .. }),
                "identity must PASS ensures result == x, got {:?}",
                v.verdict
            );
            assert_eq!(v.basis, ProofBasis::Implementation);
        }

        // Sanity: a straight-line return that violates the clause is
        // refuted, not merely Unknown. `wrong_abs` returns `x` but
        // claims `result >= 0`.
        #[test]
        fn straight_line_return_violating_clause_is_refuted() {
            let src = r#"
                fn wrong_abs(int x) -> int
                    ensures result >= 0
                { return x; }
            "#;
            let (prog, _) = parse(src);
            let verdicts = verify_program(&prog);
            let v = verdict_for(&verdicts, "wrong_abs", ClauseKind::Ensures);
            assert!(
                matches!(v.verdict, Verdict::Fail { .. }),
                "wrong_abs must FAIL ensures result >= 0, got {:?}",
                v.verdict
            );
        }
    }

    // RES-3969: basis routing is independent of z3 — it reflects
    // whether the body was in the substitution subset, not whether the
    // solver was linked. These run in every build configuration.
    #[test]
    fn ensures_basis_is_implementation_for_straight_line_body() {
        let src = "fn f(int x) -> int ensures result >= 0 { return x; }";
        let (prog, _) = parse(src);
        let v = verify_program(&prog);
        let e = v
            .iter()
            .find(|c| c.kind == ClauseKind::Ensures)
            .expect("ensures verdict");
        assert_eq!(e.basis, ProofBasis::Implementation);
    }

    #[test]
    fn ensures_basis_is_clause_only_for_out_of_subset_body() {
        // Multi-statement body with a `let` — outside the modelled
        // subset — falls back to the free-variable clause-only path.
        let src = "fn f(int x) -> int ensures result >= 0 { let t = x + 1; return t; }";
        let (prog, _) = parse(src);
        let v = verify_program(&prog);
        let e = v
            .iter()
            .find(|c| c.kind == ClauseKind::Ensures)
            .expect("ensures verdict");
        assert_eq!(e.basis, ProofBasis::ClauseOnly);
    }

    #[test]
    fn ensures_not_mentioning_result_stays_clause_only() {
        // `ensures x >= 0` doesn't constrain the return value, so
        // substitution is a no-op and we keep the clause-only label.
        let src = "fn f(int x) -> int requires x >= 0 ensures x >= 0 { return x; }";
        let (prog, _) = parse(src);
        let v = verify_program(&prog);
        let e = v
            .iter()
            .find(|c| c.kind == ClauseKind::Ensures)
            .expect("ensures verdict");
        assert_eq!(e.basis, ProofBasis::ClauseOnly);
    }
}
