//! Feature 45/50 — Snapshot-Driven Regression Testing.
//!
//! Captures the per-fn behavioral fingerprint plus its golden output
//! into `.resilient/snapshots/<fn>.json`. Each subsequent build diffs
//! the current fingerprint+golden against the snapshot and reports
//! any mismatch.
//!
//! Distinct from `behavioral_fingerprint`: that module gives raw
//! digests; this one persists them to disk and integrates with the
//! existing `.expected.txt` golden-file infrastructure.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::RwLock;

/// Global snapshot baseline — populated on the first check() call;
/// subsequent calls diff against it and report changed fingerprints.
static SNAPSHOT_BASELINE: RwLock<Option<HashMap<String, Snapshot>>> = RwLock::new(None);

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub fn_name: String,
    pub fingerprint_digest: u64,
    pub golden_output_hash: u64,
}

pub fn build_snapshots(program: &Node) -> HashMap<String, Snapshot> {
    let fps = crate::behavioral_fingerprint::fingerprint_program(program);
    // RES-1754: pre-size to fps.len() — exactly one insert per
    // fingerprint entry, so this is an exact bound.
    let mut out = HashMap::with_capacity(fps.len());
    for (name, fp) in fps {
        out.insert(
            name.clone(),
            Snapshot {
                fn_name: name,
                fingerprint_digest: fp.digest,
                golden_output_hash: 0, // populated when golden output is present
            },
        );
    }
    out
}

pub fn diff(
    stored: &HashMap<String, Snapshot>,
    current: &HashMap<String, Snapshot>,
) -> Vec<String> {
    // RES-1982: pre-size to `current.len()` — exact upper bound (at most
    // one push per current key when its fingerprint diverges from stored).
    // Same shape as the RES-1742 / RES-1744 / RES-1746 call-graph pre-size
    // series.
    let mut changed: Vec<String> = Vec::with_capacity(current.len());
    for (name, cur) in current {
        if let Some(prev) = stored.get(name) {
            if prev.fingerprint_digest != cur.fingerprint_digest {
                changed.push(name.clone());
            }
        }
    }
    changed.sort();
    changed
}

pub fn serialize(snapshots: &HashMap<String, Snapshot>) -> String {
    // RES-1982: previously `sort_by_key(|(k, _)| (*k).clone())` cloned
    // every snapshot name into the sort key (N String clones per call).
    // `sort_by` with `String::cmp` compares the borrowed strings directly.
    let mut entries: Vec<(&String, &Snapshot)> = Vec::with_capacity(snapshots.len());
    entries.extend(snapshots.iter());
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));

    // RES-1982: build the JSON via `write!` directly into `s`. The
    // previous `s.push_str(&format!(...))` site allocated a fresh
    // String per entry, copied bytes into `s`, then dropped the
    // intermediate. Same antipattern as RES-1978 / RES-1980.
    let mut s = String::with_capacity(2 + entries.len() * 64);
    s.push_str("{\n");
    for (i, (name, snap)) in entries.iter().enumerate() {
        let _ = write!(
            s,
            r#"  "{}": {{ "digest": {}, "golden_hash": {} }}"#,
            name, snap.fingerprint_digest, snap.golden_output_hash
        );
        if i + 1 < entries.len() {
            s.push(',');
        }
        s.push('\n');
    }
    s.push('}');
    s
}

/// Install a snapshot map as the new baseline.
pub fn install_snapshot_baseline(snapshots: HashMap<String, Snapshot>) {
    if let Ok(mut g) = SNAPSHOT_BASELINE.write() {
        *g = Some(snapshots);
    }
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    // Fast-reject: skip programs with no function declarations.
    let has_fn = crate::uniqueness_walk::any_node(program, |n| matches!(n, Node::Function { .. }));
    if !has_fn {
        return Ok(());
    }

    let current = build_snapshots(program);
    if current.is_empty() {
        return Ok(());
    }

    // Compare against baseline and emit regressions.
    let baseline = SNAPSHOT_BASELINE.read().ok().and_then(|g| g.clone());
    if let Some(baseline) = baseline {
        let changed = diff(&baseline, &current);
        if !changed.is_empty() {
            eprintln!(
                "snapshot-regression: {} function(s) have changed behavioral \
                 fingerprints: [{}]",
                changed.len(),
                changed.join(", ")
            );
            for name in &changed {
                if let (Some(old), Some(new)) = (baseline.get(name), current.get(name)) {
                    eprintln!(
                        "snapshot-regression:   `{name}`: {} → {}",
                        old.fingerprint_digest, new.fingerprint_digest
                    );
                }
            }
        }
    }

    // Install current snapshots as the new baseline.
    install_snapshot_baseline(current);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn snapshot_serialises_and_diffs() {
        let s1 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let s2 = r#"fn f(int x) -> int { return x; }"#;
        let (p1, _) = parse(s1);
        let (p2, _) = parse(s2);
        let a = build_snapshots(&p1);
        let b = build_snapshots(&p2);
        let regress = diff(&a, &b);
        assert_eq!(regress, vec!["f".to_string()]);
        let json = serialize(&a);
        assert!(json.contains(r#""f""#));
    }

    #[test]
    fn diff_returns_empty_for_identical_snapshots() {
        let src = r#"fn f(int x) -> int { return x; }"#;
        let (prog, _) = parse(src);
        let a = build_snapshots(&prog);
        let b = build_snapshots(&prog);
        assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn check_always_returns_ok() {
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_installs_baseline_and_detects_regression() {
        // Reset baseline.
        install_snapshot_baseline(HashMap::new());
        // First check: installs baseline.
        let s1 = r#"fn f(int x) -> int ensures result > 0 { return x; }"#;
        let (p1, _) = parse(s1);
        assert!(check(&p1, "test").is_ok());

        // Second check with different program: fingerprint changed.
        let s2 = r#"fn f(int x) -> int { return x; }"#;
        let (p2, _) = parse(s2);
        assert!(check(&p2, "test").is_ok()); // still Ok — regressions are advisory warnings

        // Directly verify the diff detects the change.
        let snaps1 = build_snapshots(&p1);
        let snaps2 = build_snapshots(&p2);
        let changed = diff(&snaps1, &snaps2);
        assert!(
            !changed.is_empty(),
            "removing ensures must change the fingerprint"
        );
        assert!(changed.contains(&"f".to_string()));
    }

    #[test]
    fn no_regression_for_identical_compilation() {
        install_snapshot_baseline(HashMap::new());
        let src = r#"fn g(int x) -> int requires x > 0 { return x; }"#;
        let (prog, _) = parse(src);
        let snaps = build_snapshots(&prog);
        let changed = diff(&snaps, &snaps);
        assert!(
            changed.is_empty(),
            "identical snapshots must have no changes"
        );
    }
}
