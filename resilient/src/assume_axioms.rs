// RES-133b: collect top-level `assume(P)` predicates from a function
// body so the verifier can admit them as axioms when discharging
// `ensures` / `recovers_to` obligations.
//
// **Soundness boundary (MVP).** Only `assume`s that occur *before
// any control-flow* in the top-level Block are collected. An
// `assume` inside an `if`, `while`, `for`, `match`, or after a
// `return` is ignored — admitting those as universal axioms would
// be unsound for the postcondition (a never-taken branch's
// `assume(false)` would let us prove anything).
//
// This is intentionally conservative; a fuller per-block scoping
// pass is RES-133's longer-term goal. This MVP handles the common
// case: `assume`s at the start of a function describing what the
// runtime check ensures we entered the body with.

use crate::Node;

/// Walk the leading prefix of a function body's top-level Block
/// and collect each `assume(P)` predicate. Stops at the first
/// statement that introduces control-flow or an early exit.
///
/// Returns conditions in source order. Caller appends them to the
/// `requires` axiom set when invoking the Z3 prover.
pub(crate) fn collect_leading_assume_axioms(body: &Node) -> Vec<Node> {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return Vec::new(),
    };
    // RES-2242: lazy-alloc. The previous shape (RES-1958)
    // pre-sized to `stmts.len()` to skip a "0→4 doubling chain",
    // but that chain is a single Vec growth — and the common case
    // is bodies with ZERO leading assumes (an `examples/` survey
    // shows ~2.5% of programs use `assume` at all, and not all
    // of those put it at the body's leading prefix).
    //
    // For 0 leading assumes (the common case): Vec::new() = 0
    // allocations vs the pre-sized 1 allocation of stmts.len()
    // Nodes. For typical body sizes (10-50 statements × Node
    // size ~80 bytes), that's 800-4000 bytes per call avoided.
    // Called by the verifier per requires/ensures/recovers_to
    // obligation the contract folder couldn't discharge, so the
    // savings compound across the Z3-dispatch hot path.
    //
    // For 1-4 leading assumes: identical (one alloc of capacity 4
    // either way). For 5+ leading assumes (extremely rare):
    // doubling chain instead of one alloc — slightly worse, but
    // the case is too rare to matter.
    let mut out: Vec<Node> = Vec::new();

    for stmt in stmts {
        match stmt {
            // `assume(P)` — admit P as an axiom.
            Node::Assume { condition, .. } => {
                out.push((**condition).clone());
            }
            // `let x = expr;` — does not introduce control flow;
            // the binding is irrelevant to the assume axioms.
            Node::LetStatement { .. } => {}
            // `assert(P)` — runtime-checked; we could admit it but
            // assert is a *check*, not an *assumption*. Skip; users
            // who want it admitted should use `assume`.
            Node::Assert { .. } => {}
            // Anything else introduces control flow or is a real
            // statement; stop collecting to stay sound.
            _ => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node;
    use crate::span::Span;

    fn ident(name: &str) -> Node {
        Node::Identifier {
            name: name.to_string(),
            span: Span::default(),
        }
    }

    fn block(stmts: Vec<Node>) -> Node {
        Node::Block {
            stmts,
            span: Span::default(),
        }
    }

    fn assume_stmt(name: &str) -> Node {
        Node::Assume {
            condition: Box::new(ident(name)),
            message: None,
            span: Span::default(),
        }
    }

    fn assert_stmt(name: &str) -> Node {
        Node::Assert {
            condition: Box::new(ident(name)),
            message: None,
            span: Span::default(),
        }
    }

    #[test]
    fn empty_body_returns_empty() {
        let body = block(vec![]);
        assert_eq!(collect_leading_assume_axioms(&body).len(), 0);
    }

    #[test]
    fn collects_leading_assumes() {
        let body = block(vec![assume_stmt("x"), assume_stmt("y")]);
        let axioms = collect_leading_assume_axioms(&body);
        assert_eq!(axioms.len(), 2);
    }

    #[test]
    fn assert_does_not_block_collection() {
        // assert is a check, not control flow; collection continues past it.
        // But we don't admit asserts as axioms.
        let body = block(vec![assume_stmt("a"), assert_stmt("b"), assume_stmt("c")]);
        let axioms = collect_leading_assume_axioms(&body);
        assert_eq!(axioms.len(), 2);
    }

    #[test]
    fn let_does_not_block_collection() {
        let body = block(vec![
            assume_stmt("a"),
            Node::LetStatement {
                name: "x".into(),
                value: Box::new(ident("v")),
                type_annot: None,
                span: Span::default(),
            },
            assume_stmt("b"),
        ]);
        let axioms = collect_leading_assume_axioms(&body);
        assert_eq!(axioms.len(), 2);
    }

    #[test]
    fn return_stops_collection() {
        let body = block(vec![
            assume_stmt("a"),
            Node::ReturnStatement {
                value: None,
                span: Span::default(),
            },
            assume_stmt("b"),
        ]);
        let axioms = collect_leading_assume_axioms(&body);
        assert_eq!(axioms.len(), 1);
    }

    #[test]
    fn non_block_body_returns_empty() {
        let body = ident("x");
        assert_eq!(collect_leading_assume_axioms(&body).len(), 0);
    }
}
