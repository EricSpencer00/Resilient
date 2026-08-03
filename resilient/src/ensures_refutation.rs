//! RES-4218: reject `ensures` clauses that Z3 refutes against the
//! function body.
//!
//! # The hole this closes
//!
//! `typechecker.rs`'s per-clause `requires`/`ensures` pass (RES-060 /
//! RES-067) asks a *clause-only* question: is this clause a universal
//! tautology, a universal contradiction, or neither? For a
//! postcondition that constrains `result`, "neither" is the answer
//! essentially always — `result` is a free variable, so
//! `result >= x && result >= y` is satisfiable for *some* `result`
//! no matter what the body computes. A `max` that wrongly returns `x`
//! type-checks exactly like a correct one, and the violation only
//! surfaces as a runtime `Contract violation` when a caller happens to
//! pass `y > x`.
//!
//! RES-3969 already built the missing piece —
//! [`crate::contract_verify`]'s body-aware path substitutes the body's
//! return expression for `result` and proves the *grounded*
//! obligation. But that pass was only reachable through
//! `--emit-contract-cert`, and its verdicts are advisory: nothing in
//! the compile path consumed them. This module is the wiring.
//!
//! # Soundness — refutations only
//!
//! A clause is rejected **only** when both of these hold:
//!
//! * the verdict is [`Verdict::Fail`], and
//! * the basis is [`ProofBasis::Implementation`] — the body was inside
//!   [`crate::contract_verify`]'s exactly-modelled subset and its
//!   return expression was substituted for `result`.
//!
//! Under those conditions Z3 found a model of
//! `requires ∧ ¬ensures[result := body]`: a concrete assignment to the
//! parameters that satisfies every precondition and still falsifies the
//! postcondition. That is a proof the function is wrong, not a
//! heuristic. Every other outcome — `Pass`, `Unknown`, a body outside
//! the modelled subset, a solver timeout, a clause that never mentions
//! `result`, or a build without `--features z3` — leaves the program
//! accepted and the runtime check in place, exactly as before.
//!
//! Consequently this pass has no false positives by construction: it
//! cannot reject a program unless the solver produced a witness.
//!
//! # Scope
//!
//! Top-level functions and `impl`-block methods. Bodies outside the
//! `{ return E; }` / `{ if C { return T; } else { return F; } }`
//! subset fall out at [`crate::contract_verify`]'s modelling step and
//! are never reported.

use crate::Node;
use crate::contract_verify::{ProofBasis, Verdict, prove_ensures, render_clause};
use crate::span::Span;

/// RES-4218: memo for [`prove_ensures`], shared by this pass and the
/// `<partial-proof warning>` suppression in `typechecker.rs`.
///
/// Both consumers ask the same question about the same clause during a
/// single type-check — the warning site while walking the `fn`, this
/// pass afterwards from `<EXTENSION_PASSES>`. Without the memo each
/// `ensures` clause would be handed to Z3 twice. The key is a
/// *content* hash (fn name + clause + every `requires` + body, all
/// span-insensitive), so entries stay valid across programs within a
/// process — which matters for the LSP and for `cargo test`, where one
/// process type-checks thousands of distinct sources.
#[cfg(feature = "z3")]
mod memo {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    thread_local! {
        static CACHE: RefCell<HashMap<u64, (Verdict, ProofBasis)>> =
            RefCell::new(HashMap::new());
    }

    fn key(fn_name: &str, clause: &Node, requires: &[Node], body: &Node) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        fn_name.hash(&mut h);
        crate::verifier_z3::hash_node_spanless(clause, &mut h);
        requires.len().hash(&mut h);
        for r in requires {
            crate::verifier_z3::hash_node_spanless(r, &mut h);
        }
        crate::verifier_z3::hash_node_spanless(body, &mut h);
        h.finish()
    }

    pub(super) fn verdict(
        fn_name: &str,
        clause: &Node,
        requires: &[Node],
        body: &Node,
    ) -> (Verdict, ProofBasis) {
        let k = key(fn_name, clause, requires, body);
        if let Some(hit) = CACHE.with(|c| c.borrow().get(&k).cloned()) {
            return hit;
        }
        let computed = prove_ensures(clause, requires, body);
        CACHE.with(|c| c.borrow_mut().insert(k, computed.clone()));
        computed
    }
}

/// The body-aware verdict for one `ensures` clause. Memoized under
/// `--features z3`; a direct call otherwise (where [`prove_ensures`]
/// is a cheap `Unknown` and caching would only cost memory).
fn body_aware_verdict(
    fn_name: &str,
    clause: &Node,
    requires: &[Node],
    body: &Node,
) -> (Verdict, ProofBasis) {
    #[cfg(feature = "z3")]
    {
        memo::verdict(fn_name, clause, requires, body)
    }
    #[cfg(not(feature = "z3"))]
    {
        let _ = fn_name;
        prove_ensures(clause, requires, body)
    }
}

/// RES-4218: whether this `ensures` clause was *proven* against the
/// function's own body, so `typechecker.rs` can suppress the
/// `warning[partial-proof]` that the free-variable RES-060 pass would
/// otherwise emit.
///
/// Before this pass existed, every `result`-constrained postcondition
/// came back `Unknown` from the clause-only query — `result` is a free
/// variable there, so the clause is neither a tautology nor a
/// contradiction — and each one produced a "Z3 returned Unknown" line
/// even for a function whose contract is fully discharged. Reporting an
/// incomplete proof for a clause we did in fact prove trains users to
/// ignore the warning; suppressing it here makes a surviving
/// `partial-proof` on an `ensures` clause mean what it says.
///
/// `is_ensures` distinguishes the `ensures` tail of `typechecker.rs`'s
/// `requires.iter().chain(ensures.iter())` loop — a `requires` clause
/// has no body-aware reading and always keeps its warning.
pub fn discharged_against_body(
    fn_name: &str,
    clause: &Node,
    requires: &[Node],
    body: &Node,
    is_ensures: bool,
) -> bool {
    if !is_ensures {
        return false;
    }
    let (verdict, basis) = body_aware_verdict(fn_name, clause, requires, body);
    basis == ProofBasis::Implementation && matches!(verdict, Verdict::Pass { .. })
}

/// RES-4218: walk the program and reject every `ensures` clause that
/// Z3 refutes against the function's own body. Entry point wired into
/// `typechecker.rs`'s `<EXTENSION_PASSES>` block.
///
/// Returns `Ok(())` unchanged on builds without `--features z3` —
/// [`crate::contract_verify`]'s prover degrades to `Unknown` there, so
/// no clause can reach the refuted state.
pub fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    let mut refuted: Vec<String> = Vec::new();
    for s in stmts {
        match &s.node {
            Node::Function { .. } => check_function(&s.node, source_path, &mut refuted),
            Node::ImplBlock { methods, .. } => {
                for m in methods {
                    check_function(m, source_path, &mut refuted);
                }
            }
            _ => {}
        }
    }
    if refuted.is_empty() {
        Ok(())
    } else {
        Err(refuted.join("\n"))
    }
}

fn check_function(node: &Node, source_path: &str, refuted: &mut Vec<String>) {
    let Node::Function {
        name,
        requires,
        ensures,
        body,
        span: fn_span,
        ..
    } = node
    else {
        return;
    };
    if ensures.is_empty() {
        return;
    }
    for clause in ensures {
        let (verdict, basis) = body_aware_verdict(name, clause, requires, body);
        // Only an Implementation-basis refutation is a proof about the
        // function. A ClauseOnly `Fail` means the clause text is
        // self-contradictory under the preconditions — that is the
        // pre-existing RES-060 pass's business, and reporting it here
        // too would double up the diagnostic.
        if basis == ProofBasis::Implementation
            && let Verdict::Fail { counterexample } = verdict
        {
            refuted.push(format_refutation(
                source_path,
                pick_span(clause, fn_span),
                name,
                &render_clause(clause),
                counterexample.as_deref(),
            ));
        }
    }
}

/// Prefer the clause's own span so the caret lands on the `ensures`
/// line; fall back to the `fn` keyword when the clause was synthesised
/// without position info.
fn pick_span(clause: &Node, fn_span: &Span) -> Span {
    let s = clause_span(clause);
    if s.start.line > 0 { s } else { *fn_span }
}

/// Best-effort span for a contract-clause expression. Contract clauses
/// are built from the identifier / literal / prefix / infix / call
/// shapes the Z3 translator models; anything else keeps the default
/// span and defers to the enclosing `fn`.
fn clause_span(clause: &Node) -> Span {
    match clause {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::CallExpression { span, .. } => *span,
        _ => Span::default(),
    }
}

fn format_refutation(
    source_path: &str,
    span: Span,
    fn_name: &str,
    clause: &str,
    counterexample: Option<&str>,
) -> String {
    let path = if source_path.is_empty() {
        "<unknown>"
    } else {
        source_path
    };
    let mut msg = format!(
        "{}:{}:{}: fn `{}` violates `ensures {}` — the body does not satisfy it for every input \
         admitted by its preconditions",
        path, span.start.line, span.start.column, fn_name, clause,
    );
    if let Some(cx) = counterexample {
        msg.push_str(&format!(" (counterexample: {})", cx));
    }
    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn check_src(src: &str) -> Result<(), String> {
        let (program, _) = parse(src);
        check(&program, "t.rz")
    }

    /// The RES-4218 repro: a `max` that always returns `x` violates
    /// `ensures result >= y` for `y > x`. With `--features z3` the
    /// grounded obligation `x >= y` is refutable, so the pass rejects.
    #[cfg(feature = "z3")]
    #[test]
    fn rejects_broken_max() {
        let err = check_src(
            "fn broken_max(int x, int y) -> int\n\
             ensures result >= x\n\
             ensures result >= y\n\
             { return x; }\n",
        )
        .expect_err("a body that ignores `y` must not satisfy `result >= y`");
        assert!(
            err.contains("broken_max") && err.contains("result >= y"),
            "diagnostic must name the fn and the refuted clause; got:\n{err}"
        );
        // `ensures result >= x` holds for `return x;` — only the second
        // clause is refuted, so exactly one line is reported.
        assert_eq!(
            err.lines().count(),
            1,
            "only the refuted clause should be reported; got:\n{err}"
        );
    }

    /// The correct `max` — both branches discharge both clauses, so the
    /// case split proves rather than refutes.
    #[cfg(feature = "z3")]
    #[test]
    fn accepts_correct_max() {
        check_src(
            "fn max(int x, int y) -> int\n\
             ensures result >= x\n\
             ensures result >= y\n\
             { if x >= y { return x; } else { return y; } }\n",
        )
        .expect("a correct max must type-check");
    }

    /// Preconditions are axioms: `return x;` does satisfy
    /// `result >= y` once the caller is required to pass `x >= y`.
    #[cfg(feature = "z3")]
    #[test]
    fn requires_clauses_are_assumed() {
        check_src(
            "fn clamped(int x, int y) -> int\n\
             requires x >= y\n\
             ensures result >= y\n\
             { return x; }\n",
        )
        .expect("`requires x >= y` discharges `ensures result >= y` for `return x;`");
    }

    /// A refuted clause inside an `impl` method is reported too.
    #[cfg(feature = "z3")]
    #[test]
    fn rejects_refuted_impl_method() {
        let err = check_src(
            "struct S { int v }\n\
             impl S {\n\
               fn pick(int a, int b) -> int\n\
               ensures result >= b\n\
               { return a; }\n\
             }\n",
        )
        .expect_err("impl methods go through the same refutation pass");
        assert!(err.contains("pick"), "got:\n{err}");
    }

    /// A body outside the modelled subset stays `ClauseOnly` — no
    /// substitution happened, so nothing may be refuted even though the
    /// function is in fact wrong.
    #[test]
    fn out_of_subset_body_is_not_refuted() {
        check_src(
            "fn helper(int x) -> int { return x; }\n\
             fn wrapped(int x, int y) -> int\n\
             ensures result >= y\n\
             { return helper(x); }\n",
        )
        .expect("a call-returning body is outside the modelled subset");
    }

    /// A clause that never mentions `result` keeps the clause-only
    /// basis, so this pass leaves it entirely to RES-060.
    #[test]
    fn clause_without_result_is_untouched() {
        check_src(
            "fn f(int x) -> int\n\
             ensures x == x\n\
             { return x; }\n",
        )
        .expect("input-only clauses are not this pass's business");
    }

    /// Without `--features z3` the prover degrades to `Unknown`, so
    /// even the textbook broken `max` still compiles and the runtime
    /// contract check remains the only backstop — exactly the
    /// pre-RES-4218 behaviour. Guards against the pass acquiring a
    /// non-solver rejection path that would fire in the default build.
    #[cfg(not(feature = "z3"))]
    #[test]
    fn no_z3_build_refutes_nothing() {
        check_src(
            "fn broken_max(int x, int y) -> int\n\
             ensures result >= y\n\
             { return x; }\n",
        )
        .expect("without z3 nothing can be refuted");
    }

    /// Functions with no `ensures` are skipped before any prover call.
    #[test]
    fn no_ensures_is_a_noop() {
        check_src("fn f(int x) -> int { return x; }\n").expect("nothing to prove");
    }

    /// Non-program nodes are ignored rather than panicking.
    #[test]
    fn non_program_node_is_ignored() {
        let node = Node::IntegerLiteral {
            value: 1,
            span: Span::default(),
        };
        check(&node, "t.rz").expect("only `Node::Program` is walked");
    }

    /// Pull one function's `(name, ensures, requires, body)` out of a
    /// source string, for the `discharged_against_body` cases.
    fn fn_parts(src: &str) -> (String, Vec<Node>, Vec<Node>, Node) {
        let (prog, _) = parse(src);
        let Node::Program(stmts) = prog else {
            panic!("not a program")
        };
        for s in stmts {
            if let Node::Function {
                name,
                ensures,
                requires,
                body,
                ..
            } = s.node
            {
                return (name, ensures, requires, *body);
            }
        }
        panic!("no function in source")
    }

    /// A fully-discharged postcondition must not also be reported as an
    /// incomplete proof — the clause-only query answers Unknown for
    /// every `result`-constrained clause, so without this the correct
    /// `max` emitted two bogus `partial-proof` warnings.
    #[cfg(feature = "z3")]
    #[test]
    fn proven_ensures_suppresses_the_partial_proof_warning() {
        let (name, ensures, requires, body) = fn_parts(
            "fn max(int x, int y) -> int\n\
             ensures result >= x\n\
             ensures result >= y\n\
             { if x >= y { return x; } else { return y; } }\n",
        );
        for clause in &ensures {
            assert!(
                discharged_against_body(&name, clause, &requires, &body, true),
                "every clause of a correct max is proven against the body"
            );
        }
    }

    /// A clause the body does *not* satisfy keeps its warning — the
    /// suppression is tied to a proof, not to the pass having looked.
    #[cfg(feature = "z3")]
    #[test]
    fn refuted_ensures_is_not_treated_as_discharged() {
        let (name, ensures, requires, body) = fn_parts(
            "fn broken_max(int x, int y) -> int\n\
             ensures result >= y\n\
             { return x; }\n",
        );
        assert!(!discharged_against_body(
            &name,
            &ensures[0],
            &requires,
            &body,
            true
        ));
    }

    /// `requires` clauses have no body-aware reading, so the `false`
    /// flag short-circuits before any prover call.
    #[test]
    fn requires_clauses_never_suppress() {
        let (name, _ensures, requires, body) =
            fn_parts("fn f(int x) -> int requires x > 0 ensures result > 0 { return x; }");
        assert!(!discharged_against_body(
            &name,
            &requires[0],
            &requires,
            &body,
            false
        ));
    }

    /// An out-of-subset body was never proven, so its warning stands.
    #[test]
    fn out_of_subset_body_keeps_its_warning() {
        let (name, ensures, requires, body) = fn_parts(
            "fn wrapped(int x) -> int\n\
             ensures result >= x\n\
             { return helper(x); }\n",
        );
        assert!(!discharged_against_body(
            &name,
            &ensures[0],
            &requires,
            &body,
            true
        ));
    }

    /// The memo must return the same verdict it computed, not a stale
    /// or colliding entry: a second identical query is a cache hit and
    /// a differing function is a distinct key.
    #[cfg(feature = "z3")]
    #[test]
    fn memo_is_content_addressed() {
        let (good, good_ens, good_req, good_body) = fn_parts(
            "fn max(int x, int y) -> int ensures result >= y \
             { if x >= y { return x; } else { return y; } }",
        );
        assert!(discharged_against_body(
            &good,
            &good_ens[0],
            &good_req,
            &good_body,
            true
        ));
        // Repeat query — served from the memo, same answer.
        assert!(discharged_against_body(
            &good,
            &good_ens[0],
            &good_req,
            &good_body,
            true
        ));
        // Same clause text, different body → different key, and the
        // refuted body must not inherit the proven verdict.
        let (bad, bad_ens, bad_req, bad_body) =
            fn_parts("fn max(int x, int y) -> int ensures result >= y { return x; }");
        assert!(!discharged_against_body(
            &bad,
            &bad_ens[0],
            &bad_req,
            &bad_body,
            true
        ));
    }

    #[test]
    fn diagnostic_carries_position_and_counterexample() {
        let msg = format_refutation("t.rz", Span::default(), "f", "result >= y", Some("x = 1"));
        assert!(
            msg.starts_with("t.rz:0:0: fn `f` violates `ensures result >= y`"),
            "{msg}"
        );
        assert!(msg.ends_with("(counterexample: x = 1)"), "{msg}");
    }

    #[test]
    fn diagnostic_without_counterexample_omits_the_suffix() {
        let msg = format_refutation("", Span::default(), "f", "result >= y", None);
        assert!(msg.starts_with("<unknown>:"), "{msg}");
        assert!(!msg.contains("counterexample"), "{msg}");
    }
}
