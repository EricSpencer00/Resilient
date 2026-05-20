//! Feature 8/50 — Vibe-Code Autopilot.
//!
//! `rz autopilot <file>` runs the full safety-audit pipeline in one
//! pass and emits a single human-readable report. It composes:
//!
//! 1. [`crate::vibe_debt`]: per-fn signal coverage.
//! 2. [`crate::resilience_score`]: per-fn graded score.
//! 3. [`crate::contract_inference`]: suggested contracts for fns
//!    that lack them.
//! 4. [`crate::behavioral_fingerprint`]: locked-in behavioral hash
//!    so a future commit can be diffed.
//! 5. [`crate::blame_attribution`]: caller graph for downstream
//!    blame messages.
//!
//! The output is one section per fn with all five signals plus an
//! action item ("add this `requires`", "no callers — add a test",
//! etc.). Designed to be paged through.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct AutopilotEntry {
    pub function_name: String,
    pub resilience_total: u32,
    pub grade: String,
    pub vibe_signals: u32,
    pub inferred_requires: Vec<String>,
    pub inferred_ensures: Vec<String>,
    pub action_items: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct AutopilotReport {
    pub entries: Vec<AutopilotEntry>,
    pub program_debt_pct: f64,
}

pub fn run(program: &Node) -> AutopilotReport {
    let scores = crate::resilience_score::score_program(program);
    let vibe = crate::vibe_debt::analyze(program);
    let inferred = crate::contract_inference::infer_program(program);

    // RES-1758: pre-size to scores.len() — exactly one push per
    // score entry, exact bound.
    let mut entries = Vec::with_capacity(scores.len());
    for s in &scores {
        let v = vibe
            .entries
            .iter()
            .find(|v| v.function_name == s.function_name);
        let inf = inferred.iter().find(|i| i.function_name == s.function_name);
        let mut actions = Vec::new();
        if s.contracts_pts < 40 {
            actions.push("add `requires` and `ensures` clauses".to_string());
        }
        if s.coverage_pts == 0 {
            actions.push("function has no callers — add a test or remove it".to_string());
        }
        if s.live_pts == 0 && s.contracts_pts < 40 {
            actions.push("consider wrapping risky calls in a `live { }` block".to_string());
        }
        entries.push(AutopilotEntry {
            function_name: s.function_name.clone(),
            resilience_total: s.total,
            grade: s.grade().to_string(),
            vibe_signals: v.map(|v| v.signals_present()).unwrap_or(0),
            inferred_requires: inf.map(|i| i.requires.clone()).unwrap_or_default(),
            inferred_ensures: inf.map(|i| i.ensures.clone()).unwrap_or_default(),
            action_items: actions,
        });
    }
    AutopilotReport {
        entries,
        program_debt_pct: vibe.debt_percent,
    }
}

pub fn format_report(report: &AutopilotReport) -> String {
    // RES-2256: write directly into `s` via `std::fmt::Write` instead
    // of the `push_str(&format!(...))` antipattern. Each `format!()`
    // previously allocated an intermediate `String` only to be
    // immediately `push_str`'d into `s`. For an N-entry report with
    // K requires/ensures/actions per entry, that's ~N*(4+K) avoidable
    // String allocations. Mirrors RES-1912 (string concatenation),
    // RES-2254 (string_interp), and the diag.rs RES-1980 rewrite.
    use std::fmt::Write;
    let mut s = String::new();
    let _ = write!(
        s,
        "Autopilot report — program-wide vibe debt: {:.1}%\n\n",
        report.program_debt_pct
    );
    for e in &report.entries {
        let _ = write!(
            s,
            "fn {}\n  resilience: {} / 100   {}\n  vibe-signals: {} / 4\n",
            e.function_name, e.resilience_total, e.grade, e.vibe_signals
        );
        for r in &e.inferred_requires {
            let _ = writeln!(s, "  suggested: requires {}", r);
        }
        for r in &e.inferred_ensures {
            let _ = writeln!(s, "  suggested: ensures {}", r);
        }
        for a in &e.action_items {
            let _ = writeln!(s, "  action: {}", a);
        }
        s.push('\n');
    }
    s
}

pub(crate) fn check(program: &Node, _source_path: &str) -> Result<(), String> {
    let has_fn = crate::uniqueness_walk::any_node(program, |n| matches!(n, Node::Function { .. }));
    if !has_fn {
        return Ok(());
    }
    let report = run(program);
    if report.entries.is_empty() {
        return Ok(());
    }
    eprintln!(
        "autopilot: program-wide vibe debt {:.1}%",
        report.program_debt_pct
    );
    for e in &report.entries {
        if !e.action_items.is_empty() {
            eprintln!(
                "autopilot: `{}` [{}] — {}",
                e.function_name,
                e.grade,
                e.action_items.join("; ")
            );
        }
        for r in &e.inferred_requires {
            eprintln!("autopilot: `{}` suggested: requires {}", e.function_name, r);
        }
        for r in &e.inferred_ensures {
            eprintln!("autopilot: `{}` suggested: ensures {}", e.function_name, r);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn report_lists_fns_with_actions() {
        let src = r#"
            fn vibe(int a, int b) { return a + b; }
        "#;
        let (prog, _) = parse(src);
        let r = run(&prog);
        let entry = r
            .entries
            .iter()
            .find(|e| e.function_name == "vibe")
            .unwrap();
        assert!(
            !entry.action_items.is_empty(),
            "vibe fn should have actions"
        );
    }

    #[test]
    fn fully_specified_fn_has_few_actions() {
        let src = r#"
            pure fn safe(int a, int b) -> int
                requires a >= 0 && b >= 0
                ensures result >= 0
            { return a + b; }
            fn caller(int dummy) { let _x = safe(1, 2); return 0; }
        "#;
        let (prog, _) = parse(src);
        let r = run(&prog);
        let entry = r
            .entries
            .iter()
            .find(|e| e.function_name == "safe")
            .unwrap();
        // Verified fn shouldn't need more contracts.
        assert!(
            entry
                .action_items
                .iter()
                .all(|a| !a.contains("add `requires`"))
        );
    }

    #[test]
    fn format_includes_program_debt() {
        let src = r#"fn f(int x) { return x; }"#;
        let (prog, _) = parse(src);
        let r = run(&prog);
        let formatted = format_report(&r);
        assert!(formatted.contains("vibe debt"));
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn check_ok_on_program_with_functions() {
        let src = r#"fn g(int x) { return x + 1; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }
}
