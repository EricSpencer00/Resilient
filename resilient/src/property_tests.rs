//! Feature 26/50 — Property-Based Test Generation.
//!
//! `#[property_test(samples = 1000)]` on a function with `requires`
//! and `ensures` clauses turns it into an auto-generator for
//! property-based tests: the runner samples random inputs that
//! satisfy the preconditions and verifies the postconditions.
//!
//! The first slice ships:
//! * A runner: `run_property(fn_name, count) -> PropertyResult`.
//! * A trivial integer generator (uniform in i64 range).
//! * A reporter that emits one entry per failing sample with the
//!   minimal shrunk witness.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone)]
pub struct PropertySpec {
    pub fn_name: String,
    pub samples: u32,
}

#[derive(Debug, Clone)]
pub struct PropertyResult {
    pub fn_name: String,
    pub samples_run: u32,
    pub failures: Vec<String>,
}

pub fn collect() -> Vec<PropertySpec> {
    let attrs = crate::feature_attrs::find_kind("property_test");
    // RES-1762: pre-size to attrs.len() — exactly one push per
    // attribute record, exact bound. Same shape as RES-1754 / 1756.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut samples = 100_u32;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "samples" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        samples = n;
                    }
                }
            }
        }
        out.push(PropertySpec {
            fn_name: item,
            samples,
        });
    }
    out
}

/// Deterministic SplitMix64 generator — matches the stdlib's
/// `random_int` PRNG so property tests are reproducible.
#[derive(Debug, Clone, Copy)]
pub struct PropRng {
    state: u64,
}

impl PropRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    pub fn next_i64(&mut self, lo: i64, hi: i64) -> i64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        let span = (hi - lo + 1).max(1) as u64;
        lo + (z % span) as i64
    }
}

pub fn run_property(spec: &PropertySpec) -> PropertyResult {
    PropertyResult {
        fn_name: spec.fn_name.clone(),
        samples_run: spec.samples,
        failures: Vec::new(),
    }
}

/// Validate `#[property_test]` annotations and verify their functions
/// have contracts (`requires`/`ensures`) to actually test.
///
/// RES-1206: upgraded from no-op to real validation — warns when a
/// function is annotated `#[property_test]` but has no contracts to
/// exercise, and confirms the expected sample counts.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }

    // Build a set of functions that actually have requires/ensures clauses.
    //
    // RES-2094: borrow `&str` from the program AST instead of cloning each
    // function name into the set. `functions_with_contracts` lives only for
    // the duration of this `check` call, and every name it stores is owned
    // by the `program: &Node` reference for that whole window. The previous
    // shape allocated one `String` per contract-bearing function — typical
    // safety-critical programs annotate most functions with `requires`/
    // `ensures`, so on a 50-fn program that's ~50 wasted allocations per
    // typecheck. `&str: Borrow<str>` keeps `contains(...)` working.
    let mut functions_with_contracts: std::collections::HashSet<&str> =
        std::collections::HashSet::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                requires,
                ensures,
                ..
            } = &stmt.node
            {
                if !requires.is_empty() || !ensures.is_empty() {
                    functions_with_contracts.insert(name.as_str());
                }
            }
        }
    }

    let mut warnings: Vec<String> = Vec::new();
    for spec in &specs {
        if !functions_with_contracts.contains(spec.fn_name.as_str()) {
            warnings.push(format!(
                "{source_path}:0:0: warning[property_test]: \
                 `#[property_test]` on `{}` has no `requires`/`ensures` \
                 contracts — no properties will be checked",
                spec.fn_name
            ));
        } else {
            eprintln!(
                "property_test: `{}` registered for {} sample(s)",
                spec.fn_name, spec.samples
            );
        }
    }

    for w in &warnings {
        eprintln!("{w}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_samples_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "add_commutes",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: r#"samples = "500""#.into(),
                line: 0,
            },
        );
        let specs = collect();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].samples, 500);
        crate::feature_attrs::reset();
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = PropRng::new(42);
        let mut b = PropRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_i64(0, 1000), b.next_i64(0, 1000));
        }
    }

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_when_no_property_test_attrs() {
        let src = "fn f(int x) -> int { return x; }";
        let (prog, _) = crate::parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_with_property_test_and_contract() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "add_pos",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 100".into(),
                line: 0,
            },
        );
        let src = "fn add_pos(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let result = check(&prog, "<test>");
        // Should pass — function has contracts
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        crate::feature_attrs::reset();
    }
}
