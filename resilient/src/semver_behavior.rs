//! Feature 6/50 — Semantic Versioning by Behavior.
//!
//! When a Resilient package publishes a new version, the compiler
//! classifies the behavioral diff against the previous version into
//! `MAJOR` / `MINOR` / `PATCH`:
//!
//! * **MAJOR** — any callable was removed OR any postcondition was
//!   weakened. Existing callers may break.
//! * **MINOR** — new fns or strengthened postconditions only. New
//!   guarantees, no breaks.
//! * **PATCH** — only fingerprints (bodies) changed; observable
//!   behavior is identical.
//!
//! Powered by [`crate::semantic_regression`] (signature/contract
//! diff) and [`crate::behavioral_fingerprint`] (digest diff). The CLI
//! surface is `rz semver-check OLD_DIR NEW_DIR`.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemverKind {
    Patch,
    Minor,
    Major,
}

impl SemverKind {
    pub fn label(self) -> &'static str {
        match self {
            SemverKind::Patch => "PATCH",
            SemverKind::Minor => "MINOR",
            SemverKind::Major => "MAJOR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SemverDecision {
    pub kind: SemverKind,
    pub reasons: Vec<String>,
}

pub fn classify(old_program: &Node, new_program: &Node) -> SemverDecision {
    let old_c = crate::semantic_regression::extract_contracts(old_program);
    let new_c = crate::semantic_regression::extract_contracts(new_program);
    let changes = crate::semantic_regression::diff(&old_c, &new_c);

    // RES-1758: pre-size to changes.len() + 1 — the loop below pushes
    // exactly one reason per `SemanticChange`, plus an optional
    // fingerprint-regressed line. Saves the 0→4→8 doubling chain on
    // realistic semver classifications (a typical refactor produces
    // 5–20 reasons).
    let mut reasons = Vec::with_capacity(changes.len() + 1);
    let mut kind = SemverKind::Patch;

    let old_fp = crate::behavioral_fingerprint::fingerprint_program(old_program);
    let new_fp = crate::behavioral_fingerprint::fingerprint_program(new_program);
    let regressed = crate::behavioral_fingerprint::diff_fingerprints(&old_fp, &new_fp);
    if !regressed.is_empty() {
        reasons.push(format!("fingerprint changed for: {}", regressed.join(", ")));
    }

    use crate::semantic_regression::SemanticChange;
    for c in &changes {
        match c {
            SemanticChange::Removed(n) => {
                kind = SemverKind::Major;
                reasons.push(format!("removed function `{n}`"));
            }
            SemanticChange::Weakened { function, .. } => {
                kind = SemverKind::Major;
                reasons.push(format!("weakened contract on `{function}`"));
            }
            SemanticChange::Added(n) => {
                if kind == SemverKind::Patch {
                    kind = SemverKind::Minor;
                }
                reasons.push(format!("added function `{n}`"));
            }
            SemanticChange::Strengthened { function, .. } => {
                if kind == SemverKind::Patch {
                    kind = SemverKind::Minor;
                }
                reasons.push(format!("strengthened contract on `{function}`"));
            }
        }
    }

    SemverDecision { kind, reasons }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn removed_fn_is_major() {
        let s1 = r#"fn f(int x) { return x; } fn g(int x) { return x; }"#;
        let s2 = r#"fn f(int x) { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let d = classify(&p1, &p2);
        assert_eq!(d.kind, SemverKind::Major);
    }

    #[test]
    fn weakened_postcondition_is_major() {
        let s1 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let s2 = r#"fn f(int x) -> int { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        assert_eq!(classify(&p1, &p2).kind, SemverKind::Major);
    }

    #[test]
    fn added_fn_only_is_minor() {
        let s1 = r#"fn f(int x) { return x; }"#;
        let s2 = r#"fn f(int x) { return x; } fn g(int y) { return y; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        assert_eq!(classify(&p1, &p2).kind, SemverKind::Minor);
    }

    #[test]
    fn body_only_change_is_patch() {
        let s1 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        // Same contract; same fingerprint; should be patch.
        let s2 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        assert_eq!(classify(&p1, &p2).kind, SemverKind::Patch);
    }
}
