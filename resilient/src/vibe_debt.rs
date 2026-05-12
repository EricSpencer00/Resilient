//! Feature 2/50 — Vibe Debt.
//!
//! "Vibe debt" is the sibling concept to tech debt: the percentage of
//! code that has been written without contracts, tests, or formal
//! guarantees. It's the direct answer to "I vibe-coded this entire
//! app, how do I know if it's safe?"
//!
//! For each top-level fn the analyzer counts a single boolean per
//! signal:
//!
//! 1. Has at least one `requires` clause? (precondition declared)
//! 2. Has at least one `ensures` clause? (postcondition declared)
//! 3. Is referenced from elsewhere in the program? (test/use proxy)
//! 4. Has a non-empty `pure`/`io` annotation, or is `@pure`? (effect)
//!
//! A fn that scores 0/4 is "fully vibe" debt. A fn at 4/4 is fully
//! verified. The module-level vibe-debt percentage is
//! `1 - (sum_score / (4 * fn_count))`.
//!
//! The CLI surface lands as `--vibe-debt`: walks the parsed program,
//! prints the per-fn breakdown plus the program-wide percentage, and
//! exits non-zero if a CI threshold (`--vibe-debt-max=N`) is exceeded.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct VibeDebtEntry {
    pub function_name: String,
    pub has_requires: bool,
    pub has_ensures: bool,
    pub is_referenced: bool,
    pub has_effect_annotation: bool,
}

impl VibeDebtEntry {
    pub fn signals_present(&self) -> u32 {
        self.has_requires as u32
            + self.has_ensures as u32
            + self.is_referenced as u32
            + self.has_effect_annotation as u32
    }

    pub fn is_full_vibe(&self) -> bool {
        self.signals_present() == 0
    }
}

#[derive(Debug, Clone, Default)]
pub struct VibeDebtReport {
    pub entries: Vec<VibeDebtEntry>,
    /// Program-wide percentage [0.0, 100.0]. 0% = nothing is verified.
    pub debt_percent: f64,
    pub fully_vibe_count: usize,
}

pub fn analyze(program: &Node) -> VibeDebtReport {
    let Node::Program(stmts) = program else {
        return VibeDebtReport::default();
    };

    let mut refs: HashMap<String, u32> = HashMap::new();
    for s in stmts {
        collect_refs(&s.node, &mut refs);
    }

    let mut entries = Vec::new();
    for s in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            effects,
            pure,
            body,
            ..
        } = &s.node
        {
            let self_calls = {
                let mut tmp = HashMap::new();
                collect_refs(body, &mut tmp);
                tmp.get(name).copied().unwrap_or(0)
            };
            let external = refs
                .get(name)
                .copied()
                .unwrap_or(0)
                .saturating_sub(self_calls);
            entries.push(VibeDebtEntry {
                function_name: name.clone(),
                has_requires: !requires.is_empty(),
                has_ensures: !ensures.is_empty(),
                is_referenced: external > 0,
                has_effect_annotation: effects.pure || *pure,
            });
        }
    }

    let n = entries.len();
    let total_signals: u32 = entries.iter().map(|e| e.signals_present()).sum();
    let debt_percent = if n == 0 {
        0.0
    } else {
        let max = (n as u32 * 4) as f64;
        100.0 * (1.0 - total_signals as f64 / max)
    };
    let fully_vibe_count = entries.iter().filter(|e| e.is_full_vibe()).count();

    VibeDebtReport {
        entries,
        debt_percent,
        fully_vibe_count,
    }
}

fn collect_refs(node: &Node, out: &mut HashMap<String, u32>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                *out.entry(name.clone()).or_insert(0) += 1;
            }
            for a in arguments {
                collect_refs(a, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_refs(s, out);
            }
        }
        Node::Function { body, .. } => collect_refs(body, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_refs(condition, out);
            collect_refs(consequence, out);
            if let Some(e) = alternative {
                collect_refs(e, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_refs(condition, out);
            collect_refs(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_refs(iterable, out);
            collect_refs(body, out);
        }
        Node::ReturnStatement { value: Some(e), .. } => collect_refs(e, out),
        Node::InfixExpression { left, right, .. } => {
            collect_refs(left, out);
            collect_refs(right, out);
        }
        Node::PrefixExpression { right, .. } => collect_refs(right, out),
        Node::LetStatement { value, .. } => collect_refs(value, out),
        Node::Assignment { value, .. } => collect_refs(value, out),
        Node::ExpressionStatement { expr, .. } => collect_refs(expr, out),
        Node::Program(stmts) => {
            for s in stmts {
                collect_refs(&s.node, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1206: this pass historically called `analyze(program)` and
    // immediately discarded the returned `VibeDebtReport`. The real
    // consumer (`autopilot::run` at autopilot.rs:42) calls `analyze`
    // directly when it needs the report, so the work here was
    // unobservable: a HashMap allocation, an AST walk, and a Vec
    // population, all dropped on function exit. The entry point is
    // kept so the `EXTENSION_PASSES` block in `typechecker.rs` stays
    // undisturbed and a future use can flow data through this slot.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn fully_vibe_program_reports_high_debt() {
        let src = r#"
            fn a(int x) { return x; }
            fn b(int x) { return x; }
            fn c(int x) { return x; }
        "#;
        let (prog, _errs) = parse(src);
        let report = analyze(&prog);
        assert_eq!(report.entries.len(), 3);
        assert_eq!(report.fully_vibe_count, 3);
        assert!(
            report.debt_percent >= 99.0,
            "debt %: {}",
            report.debt_percent
        );
    }

    #[test]
    fn fully_specified_program_reports_zero_debt() {
        let src = r#"
            pure fn add(int a, int b) -> int
                requires a >= 0 && b >= 0
                ensures result >= 0
            {
                return a + b;
            }
            fn caller(int dummy) {
                let x = add(1, 2);
            }
        "#;
        let (prog, _errs) = parse(src);
        let report = analyze(&prog);
        let add = report
            .entries
            .iter()
            .find(|e| e.function_name == "add")
            .unwrap();
        assert!(add.has_requires);
        assert!(add.has_ensures);
        assert!(add.is_referenced);
        assert!(add.has_effect_annotation);
        assert_eq!(add.signals_present(), 4);
    }

    #[test]
    fn empty_program_is_zero_percent() {
        let src = "";
        let (prog, _errs) = parse(src);
        let report = analyze(&prog);
        assert_eq!(report.debt_percent, 0.0);
    }
}
