//! Feature 1/50 — Resilience Score.
//!
//! A per-function quantified score (0–100) that summarises how
//! verified-and-safe a function is. The score is the weighted sum of
//! observable safety signals already produced by the typechecker:
//!
//! * **Contracts (40 pts)** — does the fn declare `requires` and/or
//!   `ensures`? An unverified fn body is a vibe-coded body.
//! * **Effect annotation (10 pts)** — `pure` declared, or `@pure`
//!   attribute applied.
//! * **Live recovery coverage (15 pts)** — does the fn body contain
//!   any `live { ... }` block? Self-healing code earns a bump.
//! * **Test coverage stand-in (15 pts)** — does any other top-level fn
//!   in the program reference this fn's name? Calls are the cheapest
//!   available proxy for "someone exercised this code".
//! * **Body simplicity (20 pts)** — fns with fewer than 30 statements
//!   earn a complexity bonus; over 100 statements zeroes the bucket.
//!
//! No new syntax. Pure analysis pass over the existing AST.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

/// One score breakdown for diagnostic / reporting purposes.
#[derive(Debug, Clone, Default)]
pub struct ResilienceScore {
    pub function_name: String,
    pub contracts_pts: u32,
    pub effects_pts: u32,
    pub live_pts: u32,
    pub coverage_pts: u32,
    pub simplicity_pts: u32,
    pub total: u32,
}

impl ResilienceScore {
    pub fn grade(&self) -> &'static str {
        match self.total {
            90..=100 => "A — formally guaranteed",
            75..=89 => "B — well-specified",
            60..=74 => "C — partially-specified",
            40..=59 => "D — vibe-coded with structure",
            _ => "F — vibe-coded, unverified",
        }
    }
}

pub fn score_program(program: &Node) -> Vec<ResilienceScore> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };

    // Build a call-reference index so we can credit a fn for being
    // called from somewhere. We only need names → reference count.
    // RES-1507: borrow each call-site name as `&str` from the AST
    // instead of cloning. Same pattern applied to `vibe_debt::analyze`
    // in this PR; mirrors RES-1495 / RES-1500 / RES-1503.
    // RES-1990: pre-size to 32 — typical program has 20-100 distinct
    // call-site identifiers; 32 fits the common case.
    let mut call_refs: HashMap<&str, u32> = HashMap::with_capacity(32);
    for s in stmts {
        collect_call_names(&s.node, &mut call_refs);
    }

    // RES-1756: pre-size to stmts.len() — one push per top-level
    // function, upper-bounded by stmts.len(). Same pattern as the
    // semantic_regression / vibe_debt pre-sizes.
    let mut out = Vec::with_capacity(stmts.len());
    // RES-1990: lift the per-fn self-call HashMap outside the loop and
    // `clear()` between iterations — retains the bucket allocation
    // across all functions. Same buffer-reuse shape as RES-1988
    // (vibe_debt) and RES-1966 (isr_call_graph).
    let mut self_call_buf: HashMap<&str, u32> = HashMap::with_capacity(8);
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            body,
            effects,
            pure,
            ..
        } = &s.node
        {
            let mut score = ResilienceScore {
                function_name: name.clone(),
                ..Default::default()
            };

            if !requires.is_empty() {
                score.contracts_pts += 20;
            }
            if !ensures.is_empty() {
                score.contracts_pts += 20;
            }

            if effects.pure || *pure {
                score.effects_pts = 10;
            }

            if body_contains_live(body) {
                score.live_pts = 15;
            }

            // Subtract self-references so a recursive fn can't earn
            // coverage credit by calling itself.
            let raw_refs = call_refs.get(name.as_str()).copied().unwrap_or(0);
            self_call_buf.clear();
            let self_refs = count_self_calls_into(body, name, &mut self_call_buf);
            let external_refs = raw_refs.saturating_sub(self_refs);
            score.coverage_pts = match external_refs {
                0 => 0,
                1 => 8,
                _ => 15,
            };

            let stmt_count = count_body_statements(body);
            score.simplicity_pts = match stmt_count {
                0..=10 => 20,
                11..=29 => 15,
                30..=70 => 8,
                71..=100 => 3,
                _ => 0,
            };

            score.total = score.contracts_pts
                + score.effects_pts
                + score.live_pts
                + score.coverage_pts
                + score.simplicity_pts;
            out.push(score);
        }
    }
    out
}

fn collect_call_names<'a>(node: &'a Node, out: &mut HashMap<&'a str, u32>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                *out.entry(name.as_str()).or_insert(0) += 1;
            }
            for a in arguments {
                collect_call_names(a, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_call_names(s, out);
            }
        }
        Node::Function { body, .. } => collect_call_names(body, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_call_names(condition, out);
            collect_call_names(consequence, out);
            if let Some(e) = alternative {
                collect_call_names(e, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_call_names(condition, out);
            collect_call_names(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_call_names(iterable, out);
            collect_call_names(body, out);
        }
        Node::ReturnStatement { value: Some(e), .. } => collect_call_names(e, out),
        Node::InfixExpression { left, right, .. } => {
            collect_call_names(left, out);
            collect_call_names(right, out);
        }
        Node::PrefixExpression { right, .. } => collect_call_names(right, out),
        Node::LetStatement { value, .. } => collect_call_names(value, out),
        Node::Assignment { value, .. } => collect_call_names(value, out),
        Node::ExpressionStatement { expr, .. } => collect_call_names(expr, out),
        Node::Program(stmts) => {
            for s in stmts {
                collect_call_names(&s.node, out);
            }
        }
        _ => {}
    }
}

/// RES-1990: variant that reuses a caller-provided HashMap to avoid
/// per-fn allocations. Caller must `clear()` between invocations.
fn count_self_calls_into<'a>(node: &'a Node, target: &str, tmp: &mut HashMap<&'a str, u32>) -> u32 {
    collect_call_names(node, tmp);
    tmp.get(target).copied().unwrap_or(0)
}

fn body_contains_live(node: &Node) -> bool {
    match node {
        Node::LiveBlock { .. } => true,
        Node::Block { stmts, .. } => stmts.iter().any(body_contains_live),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            body_contains_live(consequence)
                || alternative
                    .as_ref()
                    .map(|e| body_contains_live(e))
                    .unwrap_or(false)
        }
        Node::WhileStatement { body, .. } => body_contains_live(body),
        Node::ForInStatement { body, .. } => body_contains_live(body),
        _ => false,
    }
}

fn count_body_statements(node: &Node) -> usize {
    match node {
        Node::Block { stmts, .. } => stmts.len(),
        _ => 1,
    }
}

/// Emit warnings for functions that score F (0–39/100).
///
/// A low resilience score is diagnostic, not a compile error — the
/// developer may be in the middle of adding contracts. The warning
/// surface is kept as `eprintln!` so it shows up in the build log
/// without blocking compilation.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let scores = score_program(program);
    let f_grade: Vec<&ResilienceScore> = scores.iter().filter(|s| s.total < 40).collect();
    for s in &f_grade {
        eprintln!(
            "{source_path}:0:0: warning[resilience]: \
             `{}` scores {}/100 ({}) — \
             add `requires`/`ensures` contracts to improve resilience",
            s.function_name,
            s.total,
            s.grade()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn unverified_vibe_function_grades_low() {
        let src = r#"
            fn vibe_added(int x, int y) {
                return x + y;
            }
        "#;
        let (prog, _errs) = parse(src);
        let scores = score_program(&prog);
        assert_eq!(scores.len(), 1);
        let s = &scores[0];
        assert_eq!(s.function_name, "vibe_added");
        assert_eq!(s.contracts_pts, 0);
        assert_eq!(s.live_pts, 0);
        assert!(s.total < 60, "vibe-fn should grade D or F: {}", s.total);
    }

    #[test]
    fn well_specified_function_grades_higher() {
        let src = r#"
            fn safe_div(int a, int b) -> int
                requires b != 0
                ensures result * b == a
            {
                return a / b;
            }
            fn caller(int dummy) {
                let x = safe_div(10, 2);
            }
        "#;
        let (prog, _errs) = parse(src);
        let scores = score_program(&prog);
        let s = scores
            .iter()
            .find(|s| s.function_name == "safe_div")
            .unwrap();
        assert!(s.contracts_pts >= 40);
        assert!(s.coverage_pts > 0);
        assert!(s.total > 60);
    }

    // ── check() ──────────────────────────────────────────────────────────────

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_well_specified_fn() {
        let src = r#"
            fn add(int a, int b) -> int
                requires a >= 0
                ensures result >= 0
            {
                return a + b;
            }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_even_for_f_grade_fn() {
        // check() emits a warning but always returns Ok() — it's advisory.
        let src = "fn vibe(int x) { return x; }";
        let (prog, _) = parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn grade_tier_is_descriptive() {
        let s = ResilienceScore {
            total: 95,
            ..Default::default()
        };
        assert!(s.grade().starts_with("A"));
        let s = ResilienceScore {
            total: 30,
            ..Default::default()
        };
        assert!(s.grade().starts_with("F"));
    }
}
