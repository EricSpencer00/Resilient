//! Feature 26/50 - Property-Based Test Generation.
//!
//! `#[property_test(samples = 1000)]` on a function with `requires` and
//! `ensures` clauses turns it into auto-generated property-based tests:
//! the runner samples random inputs that satisfy preconditions and then
//! verifies postconditions.
//!
//! First slice ships:
//! * runner: `run_property(fn_name, count) -> PropertyResult`
//! * trivial integer generator (uniform in i64 range)
//! * reporter emits one entry per failing sample with minimal shrunk witness

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PropertySpec {
    pub samples: u32,
}

fn property_test_diagnostic(source_path: &str, line: usize, message: &str) -> String {
    format!("{source_path}:{line}:0: error[property_test]: {message}")
}

fn parse_property_test_decl(
    source_path: &str,
    item_name: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<PropertySpec, String> {
    let mut samples = None;
    let args = rec.args.trim();

    if args.is_empty() {
        return Err(property_test_diagnostic(
            source_path,
            rec.line,
            &format!(
                "invalid #[property_test] declaration `{item_name}`: missing required `samples` field"
            ),
        ));
    }

    for raw_part in args.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{item_name}`: malformed entry ``; expected `samples = <integer>`"
                ),
            ));
        }

        let Some((key, value)) = part.split_once('=') else {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{item_name}`: malformed entry `{part}`; expected `samples = <integer>`"
                ),
            ));
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match key {
            "samples" => {
                if samples.is_some() {
                    return Err(property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: duplicate `samples` field"
                        ),
                    ));
                }

                let parsed = value.parse::<u32>().map_err(|_| {
                    property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: `samples` must be a positive integer"
                        ),
                    )
                })?;

                if parsed == 0 {
                    return Err(property_test_diagnostic(
                        source_path,
                        rec.line,
                        &format!(
                            "invalid #[property_test] declaration `{item_name}`: `samples` must be greater than zero"
                        ),
                    ));
                }

                samples = Some(parsed);
            }
            other => {
                return Err(property_test_diagnostic(
                    source_path,
                    rec.line,
                    &format!(
                        "invalid #[property_test] declaration `{item_name}`: unknown field `{other}`"
                    ),
                ));
            }
        }
    }

    let Some(samples) = samples else {
        return Err(property_test_diagnostic(
            source_path,
            rec.line,
            &format!(
                "invalid #[property_test] declaration `{item_name}`: missing required `samples` field"
            ),
        ));
    };

    Ok(PropertySpec { samples })
}

pub fn collect() -> Vec<(String, PropertySpec)> {
    let attrs = crate::feature_attrs::find_kind("property_test");
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
        out.push((item, PropertySpec { samples }));
    }

    out
}

#[derive(Debug, Clone)]
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

/// Validate `#[property_test]` annotations and verify their target functions
/// have contract clauses to exercise.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let attrs = crate::feature_attrs::find_kind("property_test");
    if attrs.is_empty() {
        return Ok(());
    }

    let mut function_names = HashSet::new();
    let mut functions_with_contracts = HashSet::new();
    if let Node::Program(stmts) = program {
        for stmt in stmts {
            if let Node::Function {
                name,
                requires,
                ensures,
                ..
            } = &stmt.node
            {
                function_names.insert(name.clone());
                if !requires.is_empty() || !ensures.is_empty() {
                    functions_with_contracts.insert(name.clone());
                }
            }
        }
    }

    let mut seen = HashSet::new();
    for (fn_name, rec) in attrs {
        if !function_names.contains(fn_name.as_str()) {
            continue;
        }

        if !seen.insert(fn_name.clone()) {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{fn_name}`: duplicate declaration for this function"
                ),
            ));
        }

        let _spec = parse_property_test_decl(source_path, &fn_name, &rec)?;

        if !functions_with_contracts.contains(fn_name.as_str()) {
            return Err(property_test_diagnostic(
                source_path,
                rec.line,
                &format!(
                    "invalid #[property_test] declaration `{fn_name}`: function must declare at least one `requires` or `ensures` clause"
                ),
            ));
        }
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
        assert_eq!(specs[0].0, "add_commutes");
        assert_eq!(specs[0].1.samples, 500);
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
        assert!(result.is_ok(), "expected ok, got: {:?}", result);
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_malformed_property_test_entry() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_entry",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples 100".into(),
                line: 0,
            },
        );
        let src = "fn bad_entry(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected malformed declaration error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `bad_entry`: malformed entry `samples 100`; expected `samples = <integer>`"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_missing_samples_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "missing_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "".into(),
                line: 0,
            },
        );
        let src = "fn missing_samples(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected missing field error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `missing_samples`: missing required `samples` field"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_property_test_without_contracts() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "no_contracts",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25".into(),
                line: 0,
            },
        );
        let src = "fn no_contracts(int x) -> int { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected missing contracts error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `no_contracts`: function must declare at least one `requires` or `ensures` clause"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_property_test_declaration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "duplicate",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25".into(),
                line: 0,
            },
        );
        crate::feature_attrs::record(
            "duplicate",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 50".into(),
                line: 0,
            },
        );
        let src = "fn duplicate(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected duplicate declaration error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `duplicate`: duplicate declaration for this function"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_samples_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "duplicate_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25, samples = 50".into(),
                line: 0,
            },
        );
        let src = "fn duplicate_samples(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected duplicate samples error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `duplicate_samples`: duplicate `samples` field"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_zero_sample_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "zero_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 0".into(),
                line: 0,
            },
        );
        let src = "fn zero_samples(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected zero samples error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `zero_samples`: `samples` must be greater than zero"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_unknown_property_test_field() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "unknown_field",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = 25, fuzz = 1".into(),
                line: 0,
            },
        );
        let src = "fn unknown_field(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected unknown field error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `unknown_field`: unknown field `fuzz`"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_non_integer_sample_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "bad_samples",
            crate::feature_attrs::AttrRecord {
                name: "property_test".into(),
                args: "samples = nope".into(),
                line: 0,
            },
        );
        let src = "fn bad_samples(int x) requires x > 0 ensures result > 0 { return x + 1; }";
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "<test>").expect_err("expected non-integer samples error");
        assert_eq!(
            err,
            "<test>:0:0: error[property_test]: invalid #[property_test] declaration `bad_samples`: `samples` must be a positive integer"
        );
        crate::feature_attrs::reset();
    }
}
