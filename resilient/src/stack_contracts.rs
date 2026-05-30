//! RES-2627: Static stack usage analysis.
//!
//! `#[stack(bytes = 256)]` declares the maximum stack a fn may use.
//! `analyze()` computes per-function worst-case stack usage by
//! walking the full cross-function call graph rather than just
//! counting local call depth. Recursive cycles are flagged as
//! `Unbounded`; the deepest acyclic call chain is reported for every
//! bounded function.
//!
//! The `rz stack-usage <file>` subcommand surfaces this report on
//! stdout. The typechecker also calls `check()` to enforce
//! `#[stack(bytes=N)]` budgets.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::Node;
use std::collections::{HashMap, HashSet};

/// Per-frame overhead estimate: 64 bytes covers a modest set of
/// local variables and saved registers on Cortex-M4F.
pub const FRAME_BYTES: u64 = 64;

// ---------------------------------------------------------------------------
// Public report types
// ---------------------------------------------------------------------------

/// Maximum stack usage estimate for one function.
#[derive(Debug, Clone)]
pub struct FunctionStackReport {
    pub fn_name: String,
    /// `None` = recursive cycle detected (unbounded stack).
    pub max_bytes: Option<u64>,
    /// Call depth (frames), or `None` for unbounded.
    pub max_depth: Option<u64>,
    /// Declared budget from `#[stack(bytes = N)]`, if present.
    pub budget_bytes: Option<u64>,
    /// Deepest call chain from this function. Empty for leaf functions.
    pub deepest_path: Vec<String>,
}

impl FunctionStackReport {
    pub fn is_over_budget(&self) -> bool {
        match (self.max_bytes, self.budget_bytes) {
            (Some(used), Some(budget)) => used > budget,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// `#[stack(bytes = N)]` attribute collector
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StackSpec {
    pub fn_name: String,
    pub budget_bytes: u64,
}

pub fn collect() -> Vec<StackSpec> {
    let attrs = crate::feature_attrs::find_kind("stack");
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut budget_bytes: Option<u64> = None;
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "bytes" {
                    if let Ok(n) = v.trim().trim_matches('"').parse() {
                        budget_bytes = Some(n);
                        break;
                    }
                }
            }
        }
        if let Some(n) = budget_bytes {
            out.push(StackSpec {
                fn_name: item,
                budget_bytes: n,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Call graph construction
// ---------------------------------------------------------------------------

/// Collect the set of user-defined function names called directly in `node`.
fn direct_callees(node: &Node, out: &mut HashSet<String>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                out.insert(name.clone());
            }
            for a in arguments {
                direct_callees(a, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                direct_callees(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            direct_callees(condition, out);
            direct_callees(consequence, out);
            if let Some(alt) = alternative {
                direct_callees(alt, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            direct_callees(condition, out);
            direct_callees(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            direct_callees(iterable, out);
            direct_callees(body, out);
        }
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            direct_callees(value, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            direct_callees(expr, out);
        }
        Node::ReturnStatement { value: Some(e), .. } => {
            direct_callees(e, out);
        }
        Node::InfixExpression { left, right, .. } => {
            direct_callees(left, out);
            direct_callees(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            direct_callees(right, out);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Cross-function depth computation (with cycle detection)
// ---------------------------------------------------------------------------

/// Worst-case depth from `fn_name` through the call graph.
///
/// Returns `(depth, path)` where `path` is the chain of function names
/// from `fn_name` to the deepest leaf, or `None` if a recursive cycle
/// makes the depth unbounded.
fn compute_depth(
    fn_name: &str,
    callees: &HashMap<String, HashSet<String>>,
    // Memoized results: None = unbounded, Some((depth, path)) = bounded.
    memo: &mut HashMap<String, Option<(u64, Vec<String>)>>,
    // Functions currently on the DFS stack — cycle detection.
    on_stack: &mut HashSet<String>,
) -> Option<(u64, Vec<String>)> {
    if let Some(result) = memo.get(fn_name) {
        return result.clone();
    }
    if on_stack.contains(fn_name) {
        // Recursive cycle detected.
        return None;
    }
    on_stack.insert(fn_name.to_string());

    let children = match callees.get(fn_name) {
        Some(c) => c.clone(),
        None => {
            // Leaf function — depth 1.
            on_stack.remove(fn_name);
            let result = Some((1, vec![]));
            memo.insert(fn_name.to_string(), result.clone());
            return result;
        }
    };

    let mut best: Option<(u64, Vec<String>)> = Some((1, vec![]));
    for callee in &children {
        // Only user-defined functions are in the call graph; skip builtins.
        if !callees.contains_key(callee.as_str()) {
            continue;
        }
        match compute_depth(callee, callees, memo, on_stack) {
            None => {
                // Callee is unbounded → caller is unbounded too.
                best = None;
                break;
            }
            Some((child_depth, mut child_path)) => {
                let candidate_depth = child_depth + 1;
                if best
                    .as_ref()
                    .map(|(d, _)| candidate_depth > *d)
                    .unwrap_or(false)
                {
                    child_path.insert(0, callee.clone());
                    best = Some((candidate_depth, child_path));
                }
            }
        }
    }
    on_stack.remove(fn_name);
    memo.insert(fn_name.to_string(), best.clone());
    best
}

// ---------------------------------------------------------------------------
// Public analysis API
// ---------------------------------------------------------------------------

/// Analyse every user-defined function in `program` and return a
/// [`FunctionStackReport`] for each one.
pub fn analyze(program: &Node) -> Vec<FunctionStackReport> {
    let Node::Program(stmts) = program else {
        return vec![];
    };

    // Collect function bodies.
    let mut bodies: HashMap<String, &Node> = HashMap::new();
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            bodies.insert(name.clone(), body.as_ref());
        }
    }

    // Build the call graph (only calls to user-defined functions matter).
    let mut callees: HashMap<String, HashSet<String>> = HashMap::new();
    for (name, body) in &bodies {
        let mut cs = HashSet::new();
        direct_callees(body, &mut cs);
        // Keep only calls that go to other user-defined functions.
        cs.retain(|c| bodies.contains_key(c.as_str()));
        callees.insert(name.clone(), cs);
    }

    // Collect `#[stack(bytes=N)]` budgets.
    let budgets: HashMap<String, u64> = collect()
        .into_iter()
        .map(|s| (s.fn_name, s.budget_bytes))
        .collect();

    // Compute depth for each function.
    let mut memo: HashMap<String, Option<(u64, Vec<String>)>> = HashMap::new();
    let mut on_stack: HashSet<String> = HashSet::new();
    let mut reports: Vec<FunctionStackReport> = Vec::with_capacity(bodies.len());

    let mut fn_names: Vec<String> = bodies.keys().cloned().collect();
    fn_names.sort();

    for fn_name in &fn_names {
        let result = compute_depth(fn_name, &callees, &mut memo, &mut on_stack);
        let (max_bytes, max_depth, deepest_path) = match result {
            None => (None, None, vec![]),
            Some((depth, path)) => (Some(depth * FRAME_BYTES), Some(depth), path),
        };
        reports.push(FunctionStackReport {
            fn_name: fn_name.clone(),
            max_bytes,
            max_depth,
            budget_bytes: budgets.get(fn_name).copied(),
            deepest_path,
        });
    }

    reports
}

// ---------------------------------------------------------------------------
// Typechecker integration: enforce `#[stack(bytes=N)]` budgets
// ---------------------------------------------------------------------------

pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }
    let reports = analyze(program);
    let report_map: HashMap<&str, &FunctionStackReport> =
        reports.iter().map(|r| (r.fn_name.as_str(), r)).collect();

    for spec in &specs {
        if let Some(report) = report_map.get(spec.fn_name.as_str()) {
            match report.max_bytes {
                None => {
                    return Err(format!(
                        "{}:0:0: error: `{}` has a stack budget ({} bytes) \
                         but is recursive — worst-case stack depth is unbounded",
                        source_path, spec.fn_name, spec.budget_bytes
                    ));
                }
                Some(bytes) if bytes > spec.budget_bytes => {
                    return Err(format!(
                        "{}:0:0: error: `{}` stack budget exceeded: \
                         estimated {} bytes (depth {}) > declared {} bytes",
                        source_path,
                        spec.fn_name,
                        bytes,
                        report.max_depth.unwrap_or(0),
                        spec.budget_bytes
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn shallow_function_passes() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "small",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "256""#.into(),
                line: 0,
            },
        );
        let src = r#"fn small(int x) { return x; }"#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn no_attribute_skips_check() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // No stack attribute registered — even a deeply nested function passes.
        let src = "fn deep(int x) { return deep(deep(deep(x))); }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
    }

    #[test]
    fn budget_exceeded_returns_error() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Budget: 64 bytes (= 1 frame). Any call nests at least 2 frames.
        crate::feature_attrs::record(
            "tight",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "64""#.into(),
                line: 0,
            },
        );
        // Two nested calls → depth 2 → 128 bytes > 64 budget.
        let src = "fn helper(int x) { return x; }\nfn tight(int x) { return helper(helper(x)); }\n";
        let (prog, _) = parse(src);
        let result = check(&prog, "test");
        assert!(result.is_err(), "expected budget-exceeded error, got Ok");
        let msg = result.unwrap_err();
        assert!(
            msg.contains("tight") && msg.contains("exceeded"),
            "error message should mention function name and 'exceeded': {msg}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn unknown_function_name_is_silent() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "missing_fn",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "64""#.into(),
                line: 0,
            },
        );
        let src = "fn other(int x) { return x; }\n";
        let (prog, _) = parse(src);
        assert!(check(&prog, "test").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn analyze_leaf_function_is_depth_one() {
        let src = "fn leaf(int x) { return x; }\n";
        let (prog, _) = parse(src);
        let reports = analyze(&prog);
        let r = reports.iter().find(|r| r.fn_name == "leaf").unwrap();
        assert_eq!(r.max_depth, Some(1));
        assert_eq!(r.max_bytes, Some(FRAME_BYTES));
        assert!(r.deepest_path.is_empty());
    }

    #[test]
    fn analyze_chain_depth() {
        let src = "fn a(int x) { return b(x); }\nfn b(int x) { return x; }\n";
        let (prog, _) = parse(src);
        let reports = analyze(&prog);
        let a = reports.iter().find(|r| r.fn_name == "a").unwrap();
        // a → b → leaf: depth 2
        assert_eq!(a.max_depth, Some(2));
        assert_eq!(a.max_bytes, Some(2 * FRAME_BYTES));
    }

    #[test]
    fn analyze_recursive_function_is_unbounded() {
        let src = "fn rec(int x) { return rec(x - 1); }\n";
        let (prog, _) = parse(src);
        let reports = analyze(&prog);
        let r = reports.iter().find(|r| r.fn_name == "rec").unwrap();
        assert!(r.max_depth.is_none(), "recursive fn should be unbounded");
        assert!(r.max_bytes.is_none());
    }

    #[test]
    fn recursive_with_budget_errors() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        crate::feature_attrs::record(
            "rec",
            crate::feature_attrs::AttrRecord {
                name: "stack".into(),
                args: r#"bytes = "256""#.into(),
                line: 0,
            },
        );
        let src = "fn rec(int x) { return rec(x - 1); }\n";
        let (prog, _) = parse(src);
        let result = check(&prog, "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unbounded"));
        crate::feature_attrs::reset();
    }
}
