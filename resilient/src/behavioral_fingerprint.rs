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
    let Node::Program(stmts) = program else {
        return HashMap::new();
    };
    // RES-1756: pre-size to stmts.len() — every top-level statement
    // could be a function and produce one insert. Same shape as the
    // semantic_regression / call-graph pre-sizes.
    let mut out = HashMap::with_capacity(stmts.len());
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

/// Check for behavioral regressions against a stored fingerprint
/// snapshot in `.resilient/fingerprints.lock`.
///
/// If no fingerprints file exists the check passes silently (first
/// run). If the file exists, each function whose contract digest
/// changed since the snapshot was recorded is reported as a hard
/// error so the regression surfaces in CI.
///
/// Update the snapshot with `rz fingerprint --update` after an
/// intentional contract change.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let source_dir = std::path::Path::new(source_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let fp_path = source_dir.join(".resilient").join("fingerprints.lock");
    if !fp_path.exists() {
        return Ok(());
    }
    let content = match std::fs::read_to_string(&fp_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let stored = parse_fingerprint_lock(&content);
    if stored.is_empty() {
        return Ok(());
    }
    let current = fingerprint_program(program);
    let mut regressions: Vec<String> = Vec::new();
    for (name, fp) in &current {
        if let Some(&stored_digest) = stored.get(name.as_str()) {
            if stored_digest != fp.digest {
                regressions.push(name.clone());
            }
        }
    }
    regressions.sort();
    if regressions.is_empty() {
        return Ok(());
    }
    let msgs: Vec<String> = regressions
        .iter()
        .map(|name| {
            format!(
                "{source_path}:0:0: error[fingerprint]: behavioral fingerprint of \
                 `{name}` changed — contracts or parameter types were modified; \
                 run `rz fingerprint --update` if the change is intentional"
            )
        })
        .collect();
    Err(msgs.join("\n"))
}

/// Parse `.resilient/fingerprints.lock` — lines of the form:
/// ```text
/// fn_name = 0xdeadbeef01234567
/// ```
fn parse_fingerprint_lock(s: &str) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim().to_string();
            let v = v.trim();
            let hex = v.strip_prefix("0x").unwrap_or(v);
            if let Ok(digest) = u64::from_str_radix(hex, 16) {
                out.insert(k, digest);
            }
        }
    }
    out
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

    // ── parse_fingerprint_lock ───────────────────────────────────────────────

    #[test]
    fn parse_lock_valid_entries() {
        let s = "# comment\nf = 0xdeadbeef00000001\ng = 0x0000000000000002\n";
        let m = parse_fingerprint_lock(s);
        assert_eq!(m.get("f"), Some(&0xdeadbeef00000001_u64));
        assert_eq!(m.get("g"), Some(&0x0000000000000002_u64));
    }

    #[test]
    fn parse_lock_skips_bad_lines() {
        let s = "f = not_a_hex\ng = 0xABCD\n";
        let m = parse_fingerprint_lock(s);
        assert!(!m.contains_key("f"));
        assert_eq!(m.get("g"), Some(&0xABCD_u64));
    }

    // ── check() ──────────────────────────────────────────────────────────────

    #[test]
    fn check_ok_when_no_fingerprints_file() {
        let tmp = std::env::temp_dir().join("__rz_fp_no_file_test.rz");
        std::fs::write(&tmp, b"fn f() {}").unwrap();
        let (prog, _) = parse("fn f(int x) { return x; }");
        assert!(check(&prog, tmp.to_str().unwrap()).is_ok());
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn check_ok_when_fingerprints_match() {
        let dir = std::env::temp_dir().join("__rz_fp_match_test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }";
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, src.as_bytes()).unwrap();
        let (prog, _) = parse(src);
        let fps = fingerprint_program(&prog);
        let digest = fps["f"].digest;
        let fp_dir = dir.join(".resilient");
        std::fs::create_dir_all(&fp_dir).unwrap();
        std::fs::write(
            fp_dir.join("fingerprints.lock"),
            format!("f = 0x{:016x}\n", digest).as_bytes(),
        )
        .unwrap();
        assert!(check(&prog, src_path.to_str().unwrap()).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_errors_when_fingerprint_changed() {
        let dir = std::env::temp_dir().join("__rz_fp_changed_test");
        std::fs::create_dir_all(&dir).unwrap();
        let src = "fn f(int x) -> int ensures result > 0 { return x + 1; }";
        let src_path = dir.join("main.rz");
        std::fs::write(&src_path, src.as_bytes()).unwrap();
        let (prog, _) = parse(src);
        let fp_dir = dir.join(".resilient");
        std::fs::create_dir_all(&fp_dir).unwrap();
        // Store a deliberately wrong digest
        std::fs::write(
            fp_dir.join("fingerprints.lock"),
            b"f = 0x0000000000000001\n",
        )
        .unwrap();
        let result = check(&prog, src_path.to_str().unwrap());
        assert!(result.is_err(), "expected error for changed fingerprint");
        assert!(result.unwrap_err().contains("fingerprint"));
        let _ = std::fs::remove_dir_all(&dir);
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
