//! Feature — Semantic Non-Interference via self-composition (RES-2825).
//!
//! `#[noninterference(low = "a c", high = "b")]` on a function asserts
//! that the function's result depends *only* on its `low` (public)
//! parameters — i.e. it is independent of the `high` (secret) ones.
//! Parameters not named in either list default to `high`.
//!
//! Non-interference is a **hyperproperty**: a statement about *pairs* of
//! executions ("two runs agreeing on low inputs produce equal outputs"),
//! which a single-trace logic (LTL/TLA⁺) cannot express. The standard
//! route is **self-composition** — encode two renamed copies of the
//! function and relate them. This is what we do: translate the body's
//! return expression twice, sharing the Z3 constants for `low`
//! parameters (so they are equal by construction) and giving the `high`
//! parameters fresh constants (so they are free to differ). If the two
//! outputs can still be forced unequal, the high inputs leak; if not,
//! the function is non-interferent.
//!
//! Backed by Z3 (`--features z3`); without the feature the pass is a
//! no-op, mirroring the actor-commutativity verifier. A *proved* leak is
//! a hard error with a counterexample; an *undecidable* result
//! (timeout, or a body outside the supported integer fragment) is an
//! advisory note that never blocks the build.
//!
//! ### Scope (slice 1)
//! The body must be a single `return <expr>;` where `<expr>` is integer
//! arithmetic over the parameters (`+ - * / %`, unary `-`, integer
//! literals, parameter reads). `let`-binding inlining and richer bodies
//! are follow-ups; anything outside the fragment is reported Unknown,
//! never silently proven.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct NiSpec {
    pub fn_name: String,
    pub low: Vec<String>,
    pub high: Vec<String>,
}

/// Read every `#[noninterference(...)]` attribute from the shared
/// registry into a spec.
pub fn collect() -> Vec<NiSpec> {
    let attrs = crate::feature_attrs::find_kind("noninterference");
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut spec = NiSpec {
            fn_name: item,
            low: Vec::new(),
            high: Vec::new(),
        };
        for chunk in rec.args.split(',') {
            if let Some((k, v)) = chunk.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"');
                let names = val.split_whitespace().map(str::to_string).collect();
                match key {
                    "low" => spec.low = names,
                    "high" => spec.high = names,
                    _ => {}
                }
            }
        }
        out.push(spec);
    }
    out
}

/// The `high` parameter set for `spec`: explicitly-named highs, plus any
/// parameter that is not named `low` (default-high).
fn high_set(parameters: &[(String, String)], spec: &NiSpec) -> Vec<String> {
    let low: HashSet<&str> = spec.low.iter().map(String::as_str).collect();
    let mut highs: Vec<String> = Vec::new();
    for (_ty, name) in parameters {
        if !low.contains(name.as_str()) && !highs.contains(name) {
            highs.push(name.clone());
        }
    }
    // Honour an explicit high that may not be a parameter name too.
    for h in &spec.high {
        if !highs.contains(h) {
            highs.push(h.clone());
        }
    }
    highs
}

/// Peel a function body down to its single `return <expr>;`. Returns the
/// returned expression, or `None` if the body is not exactly one return.
fn single_return(body: &Node) -> Option<&Node> {
    let stmts = match body {
        Node::Block { stmts, .. } => stmts,
        _ => return None,
    };
    if stmts.len() != 1 {
        return None;
    }
    match &stmts[0] {
        Node::ReturnStatement { value: Some(e), .. } => Some(e.as_ref()),
        _ => None,
    }
}

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for spec in &specs {
        let found = stmts.iter().find_map(|s| match &s.node {
            Node::Function {
                name,
                parameters,
                body,
                ..
            } if *name == spec.fn_name => Some((parameters, body)),
            _ => None,
        });
        let Some((parameters, body)) = found else {
            eprintln!(
                "warning: #[noninterference] references unknown fn `{}`",
                spec.fn_name
            );
            continue;
        };
        let highs = high_set(parameters, spec);
        let Some(expr) = single_return(body) else {
            advise(
                &spec.fn_name,
                "body is not a single `return <expr>;` — only the integer-arithmetic fragment is supported in this slice",
            );
            continue;
        };
        verify(source_path, &spec.fn_name, expr, &highs)?;
    }
    Ok(())
}

#[cfg(feature = "z3")]
fn verify(source_path: &str, fn_name: &str, expr: &Node, highs: &[String]) -> Result<(), String> {
    use crate::verifier_z3::{NiOutcome, prove_noninterference};
    match prove_noninterference(expr, highs) {
        NiOutcome::Independent => {
            println!(
                "verifier: fn `{fn_name}`: output is non-interferent w.r.t. high input(s) {}",
                highs.join(", ")
            );
            Ok(())
        }
        NiOutcome::Leak {
            high_var,
            lo_in,
            hi_in,
            lo_out,
            hi_out,
        } => Err(format!(
            "{source_path}:0:0: error: noninterference: fn `{fn_name}` leaks high input `{high_var}` to its public output — counterexample: with low inputs fixed, `{high_var}`={lo_in} yields {lo_out} but `{high_var}`={hi_in} yields {hi_out}"
        )),
        NiOutcome::Unknown(why) => {
            advise(fn_name, &why);
            Ok(())
        }
    }
}

#[cfg(not(feature = "z3"))]
fn verify(
    _source_path: &str,
    fn_name: &str,
    _expr: &Node,
    _highs: &[String],
) -> Result<(), String> {
    advise(
        fn_name,
        "non-interference proof requires the `z3` feature; pass --features z3 to discharge it",
    );
    Ok(())
}

fn advise(fn_name: &str, why: &str) {
    eprintln!("note: #[noninterference] `{fn_name}`: {why}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn tag(item: &str, args: &str) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "noninterference".into(),
                args: args.into(),
                line: 0,
            },
        );
    }

    #[test]
    fn collect_parses_low_and_high() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("f", r#"low = "a c", high = "b""#);
        let specs = collect();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].low, vec!["a", "c"]);
        assert_eq!(specs[0].high, vec!["b"]);
        crate::feature_attrs::reset();
    }

    #[test]
    fn high_set_defaults_unlisted_params_to_high() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        tag("f", r#"low = "a""#);
        let spec = &collect()[0];
        let params = vec![
            ("int".to_string(), "a".to_string()),
            ("int".to_string(), "b".to_string()),
        ];
        assert_eq!(high_set(&params, spec), vec!["b"]);
        crate::feature_attrs::reset();
    }

    #[test]
    fn single_return_extracts_expr() {
        let src = "fn f(int a) -> int { return a + 1; }\n";
        let (prog, _) = parse(src);
        if let Node::Program(stmts) = &prog {
            if let Node::Function { body, .. } = &stmts[0].node {
                assert!(single_return(body).is_some());
            } else {
                panic!("expected fn");
            }
        }
    }

    #[test]
    fn check_no_attrs_is_ok() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = "fn f(int a) -> int { return a; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }
}
