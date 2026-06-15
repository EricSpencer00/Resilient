//! Feature 21/50 — Probabilistic Contracts.
//!
//! `ensures result > 0 with_probability(0.99)` semantics. Encoded
//! today as an attribute `#[probabilistic(clause = "...", p = "0.99")]`
//! on a function. Stores the probabilistic obligation in a registry;
//! the runtime check accumulates statistics over multiple calls and
//! flags the function if the empirical success rate dips below the
//! claimed probability.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

/// RES-2170: dropped the redundant `fn_name: String` field. It was
/// set from the attribute's owning item name and the only reader was
/// the `install` loop, which used it as the HashMap key — so the
/// field stored exactly what the key already encoded. Same dead-field
/// pattern as RES-2106 (snapshot fn_name), RES-2110 (PhantomSpec
/// type_name), RES-2122 (Fingerprint function_name), RES-2168
/// (IntentSpec raw_args).
#[derive(Debug, Clone)]
pub struct ProbContract {
    pub clause: String,
    pub probability: f64,
    pub trials: u64,
    pub successes: u64,
}

static CONTRACTS: LazyLock<RwLock<HashMap<String, ProbContract>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<(String, ProbContract)> {
    let attrs = crate::feature_attrs::find_kind("probabilistic");
    // RES-1756: pre-size to attrs.len() — exactly one push per
    // attribute record, exact bound. Same shape as RES-1754.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut clause = String::new();
        let mut p = 1.0_f64;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                let k = k.trim();
                let v = v.trim().trim_matches('"');
                match k {
                    "clause" => clause = v.to_string(),
                    "p" => p = v.parse().unwrap_or(1.0),
                    _ => {}
                }
            }
        }
        out.push((
            item,
            ProbContract {
                clause,
                probability: p,
                trials: 0,
                successes: 0,
            },
        ));
    }
    out
}

pub fn install(contracts: Vec<(String, ProbContract)>) {
    if let Ok(mut g) = CONTRACTS.write() {
        g.clear();
        // RES-2170: move (name, contract) pairs straight from
        // `collect()` into the map. The previous shape per-contract
        // cloned `c.fn_name` to produce the key, since the field and
        // the key encoded the same string.
        g.extend(contracts);
    }
}

pub fn record_trial(fn_name: &str, success: bool) {
    if let Ok(mut g) = CONTRACTS.write() {
        if let Some(c) = g.get_mut(fn_name) {
            c.trials += 1;
            if success {
                c.successes += 1;
            }
        }
    }
}

pub fn empirical_rate(fn_name: &str) -> Option<f64> {
    CONTRACTS.read().ok().and_then(|g| {
        g.get(fn_name).and_then(|c| {
            if c.trials == 0 {
                None
            } else {
                Some(c.successes as f64 / c.trials as f64)
            }
        })
    })
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1308: gate `install` on the non-empty case — see RES-1302
    // for the wipe-on-empty race rationale; same pattern saves a
    // wasted RwLock write per compile in the common case.
    let contracts = collect();
    if contracts.is_empty() {
        return Ok(());
    }
    install(contracts);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_reports_rate() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "noisy",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result > 0", p = "0.9""#.into(),
                line: 0,
            },
        );
        install(collect());
        for _ in 0..9 {
            record_trial("noisy", true);
        }
        record_trial("noisy", false);
        let rate = empirical_rate("noisy").unwrap();
        assert!((rate - 0.9).abs() < 1e-6);
        crate::feature_attrs::reset();
    }

    #[test]
    fn empirical_rate_returns_none_for_unregistered_fn() {
        assert!(empirical_rate("totally_unknown_fn_99").is_none());
    }

    #[test]
    fn check_ok_without_attributes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int x) -> int { return x; }\n";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn multiple_contracts_tracked_independently() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fn1",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "0.95""#.into(),
                line: 1,
            },
        );
        crate::feature_attrs::record(
            "fn2",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x < 100", p = "0.80""#.into(),
                line: 2,
            },
        );
        install(collect());
        record_trial("fn1", true);
        record_trial("fn1", true);
        record_trial("fn2", true);
        record_trial("fn2", false);
        let rate1 = empirical_rate("fn1").unwrap();
        let rate2 = empirical_rate("fn2").unwrap();
        assert!((rate1 - 1.0).abs() < 1e-6, "fn1 should be 100% success");
        assert!((rate2 - 0.5).abs() < 1e-6, "fn2 should be 50% success");
        crate::feature_attrs::reset();
    }

    #[test]
    fn probability_value_parsing() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "high_confidence",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result != 0", p = "0.99""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "low_confidence",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result >= 0", p = "0.10""#.into(),
                line: 1,
            },
        );
        install(collect());
        let high = collect();
        assert_eq!(high.len(), 2);
        assert!((high[0].1.probability - 0.99).abs() < 1e-6);
        assert!((high[1].1.probability - 0.10).abs() < 1e-6);
        crate::feature_attrs::reset();
    }

    #[test]
    fn trial_accumulation_over_multiple_calls() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "flaky_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "success", p = "0.75""#.into(),
                line: 0,
            },
        );
        install(collect());

        // Simulate 100 trials: 75 successes, 25 failures
        for _ in 0..75 {
            record_trial("flaky_fn", true);
        }
        for _ in 0..25 {
            record_trial("flaky_fn", false);
        }

        let rate = empirical_rate("flaky_fn").unwrap();
        assert!(
            (rate - 0.75).abs() < 1e-6,
            "empirical rate should be 0.75 after 100 trials"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn collect_preserves_clause_and_probability() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let clause_text = "result > 0 && result < 100";
        crate::feature_attrs::record(
            "bounded_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: format!(r#"clause = "{}", p = "0.97""#, clause_text),
                line: 5,
            },
        );

        let contracts = collect();
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].0, "bounded_fn");
        assert_eq!(contracts[0].1.clause, clause_text);
        assert!((contracts[0].1.probability - 0.97).abs() < 1e-6);
        assert_eq!(contracts[0].1.trials, 0);
        assert_eq!(contracts[0].1.successes, 0);
        crate::feature_attrs::reset();
    }

    #[test]
    fn zero_trials_returns_none_empirical_rate() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "never_called",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "always true", p = "0.50""#.into(),
                line: 0,
            },
        );
        install(collect());

        // Don't call record_trial — leave trials at 0
        let rate = empirical_rate("never_called");
        assert!(
            rate.is_none(),
            "empirical_rate should return None when trials == 0"
        );
        crate::feature_attrs::reset();
    }
}
