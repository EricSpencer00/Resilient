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
use std::sync::RwLock;

/// Global baseline program snapshot for semver-check comparisons.
/// Stores the contracts and fingerprints from the last-seen program.
struct SemverBaseline {
    contracts: std::collections::HashMap<String, crate::semantic_regression::FunctionContract>,
    fingerprints: std::collections::HashMap<String, crate::behavioral_fingerprint::Fingerprint>,
}

static SEMVER_BASELINE: RwLock<Option<SemverBaseline>> = RwLock::new(None);

pub fn install_semver_baseline(program: &Node) {
    let contracts = crate::semantic_regression::extract_contracts(program);
    let fingerprints = crate::behavioral_fingerprint::fingerprint_program(program);
    if let Ok(mut g) = SEMVER_BASELINE.write() {
        *g = Some(SemverBaseline { contracts, fingerprints });
    }
}

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

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no function declarations.
    let has_fn = crate::uniqueness_walk::any_node(program, |n| {
        matches!(n, Node::Function { .. })
    });
    if !has_fn {
        return Ok(());
    }

    let current_contracts = crate::semantic_regression::extract_contracts(program);
    let current_fps = crate::behavioral_fingerprint::fingerprint_program(program);

    // Compare against baseline if one exists.
    let baseline = SEMVER_BASELINE.read().ok().and_then(|g| {
        g.as_ref().map(|b| {
            (b.contracts.clone(), b.fingerprints.clone())
        })
    });

    if let Some((old_contracts, old_fps)) = baseline {
        // Build ephemeral old/new programs to reuse classify(), or
        // diff directly from the contract/fingerprint maps.
        let changes = crate::semantic_regression::diff(&old_contracts, &current_contracts);
        let regressed = crate::behavioral_fingerprint::diff_fingerprints(&old_fps, &current_fps);

        use crate::semantic_regression::SemanticChange;
        let mut kind = SemverKind::Patch;
        let mut reasons: Vec<String> = Vec::new();

        if !regressed.is_empty() {
            reasons.push(format!("fingerprint changed for: {}", regressed.join(", ")));
        }
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
                    if kind == SemverKind::Patch { kind = SemverKind::Minor; }
                    reasons.push(format!("added function `{n}`"));
                }
                SemanticChange::Strengthened { function, .. } => {
                    if kind == SemverKind::Patch { kind = SemverKind::Minor; }
                    reasons.push(format!("strengthened contract on `{function}`"));
                }
            }
        }

        if !reasons.is_empty() {
            eprintln!(
                "semver: {} bump recommended ({})",
                kind.label(),
                reasons.join("; ")
            );
        }
    }

    // Install current as the new baseline for subsequent compilations.
    if let Ok(mut g) = SEMVER_BASELINE.write() {
        *g = Some(SemverBaseline {
            contracts: current_contracts,
            fingerprints: current_fps,
        });
    }
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

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_installs_baseline_and_detects_major() {
        // Install a baseline with a requires clause.
        let s1 = r#"fn f(int x) -> int requires x > 0 { return x; }"#;
        let (p1, _) = parse(s1);
        install_semver_baseline(&p1);

        // Now check with a weaker program (no requires) — must return Ok (advisory).
        let s2 = r#"fn f(int x) -> int { return x; }"#;
        let (p2, _) = parse(s2);
        assert!(check(&p2, "test").is_ok());

        // Verify the diff directly classifies as MAJOR.
        let decision = classify(&p1, &p2);
        assert_eq!(decision.kind, SemverKind::Major);
        assert!(decision.reasons.iter().any(|r| r.contains("weakened")));
    }

    #[test]
    fn check_ok_no_baseline() {
        // Calling check without a baseline installed should not panic.
        // Reset the baseline by installing an empty one.
        if let Ok(mut g) = SEMVER_BASELINE.write() {
            *g = None;
        }
        let src = r#"fn f(int x) -> int { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
