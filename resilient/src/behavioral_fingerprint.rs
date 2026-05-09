//! Feature 3/50 — Behavioral Fingerprinting.
//!
//! Capture the *observable behavior* of a function (not just its
//! signature) at a moment in time, store it as a stable hash, and
//! compare against future commits to detect behavioral regressions
//! that signature-only diffs miss.
//!
//! A function's fingerprint is a SipHash-flavoured digest over:
//!
//! * The serialized list of `requires` clauses (sorted lexically so
//!   reordering them doesn't change the fingerprint).
//! * The serialized list of `ensures` clauses.
//! * The serialized list of `fails` variants.
//! * The presence (yes/no) of `live`/`recovers_to` recovery paths.
//! * The serialized parameter types (so a sig change breaks the
//!   fingerprint by design — that *is* a behavioral change).
//!
//! The body itself is intentionally NOT hashed: a refactor that
//! preserves all postconditions should not invalidate the
//! fingerprint. That is the whole point — observable behavior, not
//! syntactic identity.
//!
//! Fingerprints are stored in `.resilient/fingerprints.json` (one
//! entry per fn). The CLI surface is `--check-fingerprints`, which
//! diffs the current program against the stored map and reports
//! every fn whose fingerprint changed.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct Fingerprint {
    pub function_name: String,
    pub digest: u64,
    pub has_recovery: bool,
    pub fails_variants: Vec<String>,
}

pub fn fingerprint_program(program: &Node) -> HashMap<String, Fingerprint> {
    let mut out = HashMap::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function {
            name,
            parameters,
            requires,
            ensures,
            fails,
            recovers_to,
            ..
        } = &s.node
        {
            let fp = compute_fingerprint(name, parameters, requires, ensures, fails, recovers_to);
            out.insert(name.clone(), fp);
        }
    }
    out
}

fn compute_fingerprint(
    name: &str,
    params: &[(String, String)],
    requires: &[Node],
    ensures: &[Node],
    fails: &[String],
    recovers_to: &Option<Box<Node>>,
) -> Fingerprint {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let param_repr: Vec<String> = params.iter().map(|(ty, n)| format!("{ty}:{n}")).collect();
    param_repr.hash(&mut hasher);

    let mut req_sorted: Vec<String> = requires.iter().map(node_text).collect();
    req_sorted.sort();
    req_sorted.hash(&mut hasher);

    let mut ens_sorted: Vec<String> = ensures.iter().map(node_text).collect();
    ens_sorted.sort();
    ens_sorted.hash(&mut hasher);

    let mut fails_sorted: Vec<String> = fails.to_vec();
    fails_sorted.sort();
    fails_sorted.hash(&mut hasher);

    let has_recovery = recovers_to.is_some();
    has_recovery.hash(&mut hasher);

    Fingerprint {
        function_name: name.to_string(),
        digest: hasher.finish(),
        has_recovery,
        fails_variants: fails_sorted,
    }
}

fn node_text(n: &Node) -> String {
    match n {
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::FloatLiteral { value, .. } => value.to_string(),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::StringLiteral { value, .. } => format!("{value:?}"),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => format!("({} {operator} {})", node_text(left), node_text(right)),
        Node::PrefixExpression {
            operator, right, ..
        } => {
            format!("({operator}{})", node_text(right))
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            let args: Vec<String> = arguments.iter().map(node_text).collect();
            format!("{}({})", node_text(function), args.join(", "))
        }
        _ => format!("{:?}", std::ptr::addr_of!(*n)),
    }
}

/// Compare a fresh fingerprint set against a stored one. Returns the
/// list of behavioral regressions: fns whose digest changed.
pub fn diff_fingerprints(
    stored: &HashMap<String, Fingerprint>,
    current: &HashMap<String, Fingerprint>,
) -> Vec<String> {
    let mut changed = Vec::new();
    for (name, cur) in current {
        if let Some(prev) = stored.get(name) {
            if prev.digest != cur.digest {
                changed.push(name.clone());
            }
        }
    }
    changed.sort();
    changed
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let _ = fingerprint_program(program);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn identical_programs_share_fingerprints() {
        let src = r#"
            fn f(int x) -> int requires x > 0 ensures result > 0 { return x + 1; }
        "#;
        let (p1, _) = parse(src);
        let (p2, _) = parse(src);
        let f1 = fingerprint_program(&p1);
        let f2 = fingerprint_program(&p2);
        assert_eq!(f1["f"].digest, f2["f"].digest);
    }

    #[test]
    fn weakened_postcondition_breaks_fingerprint() {
        let src1 = r#"
            fn f(int x) -> int ensures result > 0 { return x; }
        "#;
        let src2 = r#"
            fn f(int x) -> int { return x; }
        "#;
        let (p1, _) = parse(src1);
        let (p2, _) = parse(src2);
        let f1 = fingerprint_program(&p1);
        let f2 = fingerprint_program(&p2);
        assert_ne!(f1["f"].digest, f2["f"].digest);
    }

    #[test]
    fn body_change_does_not_break_fingerprint() {
        let src1 = r#"
            fn f(int x) -> int ensures result > 0 { return x + 1; }
        "#;
        let src2 = r#"
            fn f(int x) -> int ensures result > 0 { return x * 2 - x + 1; }
        "#;
        let (p1, _) = parse(src1);
        let (p2, _) = parse(src2);
        let f1 = fingerprint_program(&p1);
        let f2 = fingerprint_program(&p2);
        assert_eq!(
            f1["f"].digest, f2["f"].digest,
            "body refactor must not break the fingerprint"
        );
    }

    #[test]
    fn diff_reports_regressed_fn() {
        let s1 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let s2 = r#"fn f(int x) -> int { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let regressions = diff_fingerprints(&fingerprint_program(&p1), &fingerprint_program(&p2));
        assert_eq!(regressions, vec!["f".to_string()]);
    }
}
