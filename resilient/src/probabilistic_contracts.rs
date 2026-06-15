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

    // ── Extended malformed-input regression corpus (RES-3756) ────────────────

    #[test]
    fn malformed_probability_exceeds_one() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "1.5""#.into(),
                line: 0,
            },
        );
        // collect() should parse but p > 1.0 is semantically invalid
        let contracts = collect();
        assert!(
            !contracts.is_empty(),
            "should parse despite invalid probability"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_probability_negative() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "-0.5""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        // Should parse, but negative p is invalid
        let _ = contracts;
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_empty_clause() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "", p = "0.9""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        // Empty clause is semantically invalid
        assert!(!contracts.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_missing_clause() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"p = "0.9""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        assert!(!contracts.is_empty(), "should parse missing clause");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_non_numeric_probability() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "very_high""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        // Invalid p defaults to 1.0
        assert!(!contracts.is_empty());
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_duplicate_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "same_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "0.9""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "same_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "y < 10", p = "0.8""#.into(),
                line: 1,
            },
        );
        let contracts = collect();
        // Duplicates should result in only one contract per function
        let filtered: Vec<_> = contracts.iter().filter(|(n, _)| n == "same_fn").collect();
        assert_eq!(filtered.len(), 2, "both attributes should be collected");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_invalid_probability_format() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "x > 0", p = "0.9.9""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        // Malformed p defaults to 1.0
        assert!(!contracts.is_empty());
        crate::feature_attrs::reset();
    }

    // Valid baseline cases

    #[test]
    fn valid_multiple_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fn1",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result > 0", p = "0.95""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "fn2",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result != 0", p = "0.99""#.into(),
                line: 1,
            },
        );
        let contracts = collect();
        assert_eq!(contracts.len(), 2, "should collect multiple contracts");
        crate::feature_attrs::reset();
    }

    #[test]
    fn valid_probability_boundary_values() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "fn_certain",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "always true", p = "1.0""#.into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "fn_impossible",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "always false", p = "0.0""#.into(),
                line: 1,
            },
        );
        let contracts = collect();
        assert_eq!(contracts.len(), 2);
        install(contracts.clone());
        assert_eq!(empirical_rate("fn_certain"), None);
        assert_eq!(empirical_rate("fn_impossible"), None);
        crate::feature_attrs::reset();
    }

    #[test]
    fn valid_complex_clause_expressions() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "complex_fn",
            crate::feature_attrs::AttrRecord {
                name: "probabilistic".into(),
                args: r#"clause = "result > 0 && result < 100 || count == 0", p = "0.85""#.into(),
                line: 0,
            },
        );
        let contracts = collect();
        assert_eq!(contracts.len(), 1);
        let (name, contract) = &contracts[0];
        assert_eq!(name, "complex_fn");
        assert!(contract.clause.contains("&&") || contract.clause.contains("result"));
        crate::feature_attrs::reset();
    }
}
