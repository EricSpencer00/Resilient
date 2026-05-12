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

#[derive(Debug, Clone)]
pub struct ProbContract {
    pub fn_name: String,
    pub clause: String,
    pub probability: f64,
    pub trials: u64,
    pub successes: u64,
}

static CONTRACTS: LazyLock<RwLock<HashMap<String, ProbContract>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn collect() -> Vec<ProbContract> {
    let attrs = crate::feature_attrs::find_kind("probabilistic");
    let mut out = Vec::new();
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
        out.push(ProbContract {
            fn_name: item,
            clause,
            probability: p,
            trials: 0,
            successes: 0,
        });
    }
    out
}

pub fn install(contracts: Vec<ProbContract>) {
    if let Ok(mut g) = CONTRACTS.write() {
        g.clear();
        for c in contracts {
            g.insert(c.fn_name.clone(), c);
        }
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
}
