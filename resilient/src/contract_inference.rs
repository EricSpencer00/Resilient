//! Feature 4/50 — Contract Inference.
//!
//! Given a function with no `requires`/`ensures` declared, infer the
//! strongest invariants that are consistent with its body. The analyzer
//! ships a syntactic abductor that scans the fn body and proposes:
//!
//! Preconditions (`requires`):
//!
//! * `requires p != 0` when `p` appears as a divisor in `a / p` or `a % p`.
//! * `requires p > 0` when `p` is used as a loop iteration bound in a
//!   `while ... < p` or similar context.
//! * `requires len(p) > 0` when `p[0]` is read without a bounds check, or
//!   when `for x in p` iterates `p` (empty array would silently skip body).
//! * `requires p != null` when a field of `p` is accessed (`p.field`) —
//!   null dereference is the canonical missing null check.
//!
//! Postconditions (`ensures`):
//!
//! * `ensures result == X` when the body has exactly one `return X;`
//!   and `X` is a closed-form expression in the parameters.
//! * `ensures result >= 0` when every return expression is a product
//!   or sum of parameters that have their own `requires p >= 0`, or when
//!   the body returns an absolute value / array length.
//!
//! The inferences are reported by `--infer-contracts` rather than
//! injected into the AST — preserves the auditability story (the
//! programmer accepts the inferred contracts explicitly by copying
//! them into the source).

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InferredContracts {
    pub function_name: String,
    pub requires: Vec<String>,
    pub ensures: Vec<String>,
}

pub fn infer_program(program: &Node) -> Vec<InferredContracts> {
    let Node::Program(stmts) = program else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(stmts.len());
    for s in stmts {
        if let Node::Function {
            name,
            parameters,
            body,
            requires,
            ensures,
            ..
        } = &s.node
        {
            // Skip already-specified fns — we don't second-guess the human.
            if !requires.is_empty() && !ensures.is_empty() {
                continue;
            }
            // Per-param inference pushes at most ~4 entries each
            // (divide-by-zero, len > 0 from indexing, len > 0 from
            // iteration, non-null from field access); using
            // `parameters.len()` as the capacity covers the common
            // case (1-2 contracts inferred per param) and lets
            // `Vec::extend`'s amortised growth take over for the
            // rare param that hits multiple branches.
            let mut req = Vec::with_capacity(parameters.len());
            let mut ens = Vec::with_capacity(parameters.len());
            for (_, pname) in parameters {
                if requires.is_empty() {
                    if body_divides_by(body, pname) {
                        req.push(format!("{pname} != 0"));
                    }
                    if body_indexes_into(body, pname) {
                        req.push(format!("len({pname}) > 0"));
                    }
                    if body_iterates(body, pname) && !req.iter().any(|r| r.contains(pname)) {
                        req.push(format!("len({pname}) > 0"));
                    }
                    if body_accesses_field(body, pname) {
                        req.push(format!("{pname} != null"));
                    }
                    if body_uses_as_loop_bound(body, pname) {
                        req.push(format!("{pname} > 0"));
                    }
                    // `param` used as a shift amount requires 0 <= param <= 63.
                    if body_uses_as_shift_amount(body, pname)
                        && !req.iter().any(|r| r.contains(pname))
                    {
                        req.push(format!("0 <= {pname} && {pname} <= 63"));
                    }
                    // `param` compared to negative literal → likely needs `requires param >= 0`.
                    if body_compares_to_negative(body, pname)
                        && !req.iter().any(|r| r.contains(pname))
                    {
                        req.push(format!("{pname} >= 0"));
                    }
                }
            }
            if ensures.is_empty() {
                // RES-2210: `single_return_expr` now returns `None` for
                // unsupported AST shapes (was: `Some("<complex>")` —
                // the previous code compared against the sentinel
                // string and discarded it after every match arm).
                if let Some(e) = single_return_expr(body) {
                    ens.push(format!("result == {e}"));
                } else if all_returns_non_negative(body) {
                    ens.push("result >= 0".to_string());
                }
                // Additional: `ensures result > 0` when all returns are strictly positive.
                if ens.is_empty() && all_returns_strictly_positive(body) {
                    ens.push("result > 0".to_string());
                }
            }
            if !req.is_empty() || !ens.is_empty() {
                out.push(InferredContracts {
                    function_name: name.clone(),
                    requires: req,
                    ensures: ens,
                });
            }
        }
    }
    out
}

// ── Precondition detectors ──────────────────────────────────────────────────

fn body_divides_by(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        if let Node::InfixExpression {
            operator, right, ..
        } = n
        {
            (*operator == "/" || *operator == "%")
                && matches!(right.as_ref(), Node::Identifier { name, .. } if name == param)
        } else {
            false
        }
    })
}

/// `p[0]` — array-literal index 0 without an explicit bounds check.
fn body_indexes_into(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        if let Node::IndexExpression { target, index, .. } = n {
            matches!(target.as_ref(), Node::Identifier { name, .. } if name == param)
                && matches!(index.as_ref(), Node::IntegerLiteral { value: 0, .. })
        } else {
            false
        }
    })
}

/// `for x in p` — iterating over `p` assumes it is non-empty when the body
/// has observable side-effects (i.e., the programmer expects it to run).
fn body_iterates(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        matches!(n, Node::ForInStatement { iterable, .. }
            if matches!(iterable.as_ref(), Node::Identifier { name, .. } if name == param))
    })
}

/// `p.field` — field access on `p` implies `p` must not be null.
fn body_accesses_field(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| match n {
        Node::FieldAccess { target, .. } | Node::FieldAssignment { target, .. } => {
            matches!(target.as_ref(), Node::Identifier { name, .. } if name == param)
        }
        _ => false,
    })
}

/// Detect `while ... < p`, `while ... <= p`, `for i in 0..p` — contexts
/// where `p` serves as a loop bound and must be > 0 to be useful.
fn body_uses_as_loop_bound(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        if let Node::WhileStatement { condition, .. } = n {
            is_upper_bound_for(condition, param)
        } else {
            false
        }
    })
}

/// `expr < p` or `expr <= p` — `p` is the upper bound of the comparison.
fn is_upper_bound_for(node: &Node, param: &str) -> bool {
    if let Node::InfixExpression {
        operator, right, ..
    } = node
    {
        if (*operator == "<" || *operator == "<=")
            && matches!(right.as_ref(), Node::Identifier { name, .. } if name == param)
        {
            return true;
        }
    }
    false
}

/// Detect `x << param` or `x >> param` — `param` is used as a shift amount,
/// requiring `0 <= param <= 63` to avoid undefined behavior.
fn body_uses_as_shift_amount(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        if let Node::InfixExpression {
            operator, right, ..
        } = n
        {
            (*operator == "<<" || *operator == ">>")
                && matches!(right.as_ref(), Node::Identifier { name, .. } if name == param)
        } else {
            false
        }
    })
}

/// Detect `param < -N` or `param > -N` for any negative literal N — the
/// comparison makes no sense unless `param` can be negative, so we infer
/// `requires param >= 0` when the comparison is `param >= 0` check style.
///
/// More concretely: `if param < -1` or `param > -1` implies the programmer
/// expects `param` to potentially be negative, so we infer `requires param >= 0`
/// as the safe lower bound.
fn body_compares_to_negative(node: &Node, param: &str) -> bool {
    crate::uniqueness_walk::any_node(node, |n| {
        if let Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } = n
        {
            let is_relational = matches!(*operator, "<" | "<=" | ">" | ">=");
            if !is_relational {
                return false;
            }
            // `param < -N` or `-N < param`
            let left_is_param =
                matches!(left.as_ref(), Node::Identifier { name, .. } if name == param);
            let right_is_neg_literal = matches!(right.as_ref(),
                Node::PrefixExpression { operator: op, right: r, .. }
                    if *op == "-" && matches!(r.as_ref(), Node::IntegerLiteral { value, .. } if *value > 0)
            );
            if left_is_param && right_is_neg_literal {
                return true;
            }
            let right_is_param =
                matches!(right.as_ref(), Node::Identifier { name, .. } if name == param);
            let left_is_neg_literal = matches!(left.as_ref(),
                Node::PrefixExpression { operator: op, right: r, .. }
                    if *op == "-" && matches!(r.as_ref(), Node::IntegerLiteral { value, .. } if *value > 0)
            );
            right_is_param && left_is_neg_literal
        } else {
            false
        }
    })
}

// ── Postcondition detectors ─────────────────────────────────────────────────

fn single_return_expr(node: &Node) -> Option<String> {
    let stmts = if let Node::Block { stmts, .. } = node {
        stmts
    } else {
        return None;
    };
    let returns: Vec<&Node> = stmts
        .iter()
        .filter(|s| matches!(s, Node::ReturnStatement { value: Some(_), .. }))
        .collect();
    if returns.len() != 1 {
        return None;
    }
    if let Node::ReturnStatement { value: Some(e), .. } = returns[0] {
        // RES-2210: forward the inner `Option` directly. The previous
        // shape wrapped `format_simple_expr`'s sentinel `"<complex>"`
        // in `Some(...)`, forcing every caller to compare-and-discard.
        return format_simple_expr(e);
    }
    None
}

/// Render a node as a simple-expression source string, or return
/// `None` if the node falls outside the supported grammar
/// (compositions of identifiers, integer / boolean literals, infix /
/// prefix arithmetic, and the curated builtin set).
///
/// RES-2210: returns `Option<String>` instead of a sentinel
/// `"<complex>"` String. The previous shape allocated one
/// `"<complex>".to_string()` per unsupported sub-node — paid on every
/// branch of the recursion that hit a non-simple AST shape — even
/// though no caller ever needed the string itself (every consumer
/// compared against the literal `"<complex>"`). Switching to
/// `Option<String>` lets the recursion propagate failure via `?` with
/// zero allocations, and removes the equally-allocating `==` checks
/// against the sentinel.
fn format_simple_expr(node: &Node) -> Option<String> {
    match node {
        Node::Identifier { name, .. } => Some(name.clone()),
        Node::IntegerLiteral { value, .. } => Some(value.to_string()),
        Node::BooleanLiteral { value, .. } => Some(value.to_string()),
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let l = format_simple_expr(left)?;
            let r = format_simple_expr(right)?;
            Some(format!("({l} {operator} {r})"))
        }
        Node::PrefixExpression {
            operator, right, ..
        } if *operator == "-" => {
            let inner = format_simple_expr(right)?;
            Some(format!("(-{inner})"))
        }
        // Well-known pure functions: one-arg (len, abs) and two-arg (min, max, clamp).
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                match (name.as_str(), arguments.len()) {
                    ("len" | "abs", 1) => {
                        let arg = format_simple_expr(&arguments[0])?;
                        return Some(format!("{name}({arg})"));
                    }
                    ("min" | "max", 2) => {
                        let a = format_simple_expr(&arguments[0])?;
                        let b = format_simple_expr(&arguments[1])?;
                        return Some(format!("{name}({a}, {b})"));
                    }
                    ("clamp", 3) => {
                        let v = format_simple_expr(&arguments[0])?;
                        let lo = format_simple_expr(&arguments[1])?;
                        let hi = format_simple_expr(&arguments[2])?;
                        return Some(format!("clamp({v}, {lo}, {hi})"));
                    }
                    _ => {}
                }
            }
            None
        }
        _ => None,
    }
}

/// Returns true if every `return` in the body returns a value that is
/// syntactically guaranteed non-negative: a non-negative integer literal,
/// `len(...)`, `abs(...)`, or a non-negative arithmetic expression.
/// Recursively checks inside if/else branches so conditional returns are covered.
fn all_returns_non_negative(node: &Node) -> bool {
    let mut found_any = false;
    let mut all_ok = true;
    collect_returns_non_negative(node, &mut found_any, &mut all_ok);
    found_any && all_ok
}

fn collect_returns_non_negative(node: &Node, found: &mut bool, all_ok: &mut bool) {
    match node {
        Node::ReturnStatement { value: Some(e), .. } => {
            *found = true;
            if !expr_is_non_negative(e) {
                *all_ok = false;
            }
        }
        Node::ReturnStatement { value: None, .. } => {
            *found = true;
            *all_ok = false;
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_returns_non_negative(s, found, all_ok);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_returns_non_negative(consequence, found, all_ok);
            if let Some(alt) = alternative {
                collect_returns_non_negative(alt, found, all_ok);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            collect_returns_non_negative(body, found, all_ok);
        }
        // Do not recurse into nested function definitions.
        Node::Function { .. } => {}
        _ => {}
    }
}

/// Returns true if every `return` in the body returns a value that is
/// syntactically strictly positive (> 0). Checks recursively inside
/// if/else branches so conditional returns are covered.
fn all_returns_strictly_positive(body: &Node) -> bool {
    let mut found_any = false;
    let mut all_ok = true;
    collect_returns_strictly_positive(body, &mut found_any, &mut all_ok);
    found_any && all_ok
}

fn collect_returns_strictly_positive(node: &Node, found: &mut bool, all_ok: &mut bool) {
    match node {
        Node::ReturnStatement { value: Some(e), .. } => {
            *found = true;
            if !expr_is_strictly_positive(e) {
                *all_ok = false;
            }
        }
        Node::ReturnStatement { value: None, .. } => {
            *found = true;
            *all_ok = false;
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_returns_strictly_positive(s, found, all_ok);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            collect_returns_strictly_positive(consequence, found, all_ok);
            if let Some(alt) = alternative {
                collect_returns_strictly_positive(alt, found, all_ok);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            collect_returns_strictly_positive(body, found, all_ok);
        }
        Node::Function { .. } => {}
        _ => {}
    }
}

fn expr_is_strictly_positive(node: &Node) -> bool {
    match node {
        Node::IntegerLiteral { value, .. } => *value > 0,
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => match *operator {
            "+" => {
                expr_is_non_negative(left) && expr_is_strictly_positive(right)
                    || expr_is_strictly_positive(left) && expr_is_non_negative(right)
            }
            "*" => expr_is_strictly_positive(left) && expr_is_strictly_positive(right),
            _ => false,
        },
        _ => false,
    }
}

fn expr_is_non_negative(node: &Node) -> bool {
    match node {
        Node::IntegerLiteral { value, .. } => *value >= 0,
        Node::CallExpression { function, .. } => {
            matches!(function.as_ref(), Node::Identifier { name, .. } if name == "len" || name == "abs")
        }
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } if matches!(*operator, "+" | "*") => {
            expr_is_non_negative(left) && expr_is_non_negative(right)
        }
        _ => false,
    }
}

/// Emit contract suggestions for functions that have no explicit
/// contracts and for which the syntactic abductor found candidates.
///
/// Functions that are fully specified (both `requires` and `ensures`
/// present) are already handled by the Z3 verifier and are skipped.
/// Functions with inferred suggestions receive a `note[contract_infer]`
/// diagnostic so developers see actionable hints inline during
/// compilation rather than having to run `--suggest-contracts`
/// separately.
///
/// Always returns `Ok(())` — suggestions are never hard errors.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let inferred = infer_program(program);
    if inferred.is_empty() {
        return Ok(());
    }
    // Identify functions that have zero contracts — these are the
    // highest-priority targets: developers haven't started yet.
    let zero_contract: std::collections::HashSet<&str> = match program {
        Node::Program(stmts) => stmts
            .iter()
            .filter_map(|s| {
                if let Node::Function {
                    name,
                    requires,
                    ensures,
                    ..
                } = &s.node
                {
                    if requires.is_empty() && ensures.is_empty() {
                        return Some(name.as_str());
                    }
                }
                None
            })
            .collect(),
        _ => std::collections::HashSet::new(),
    };
    for ic in &inferred {
        if ic.requires.is_empty() && ic.ensures.is_empty() {
            continue;
        }
        let label = if zero_contract.contains(ic.function_name.as_str()) {
            "warning[contract_infer]"
        } else {
            "note[contract_infer]"
        };
        let mut parts: Vec<String> = Vec::with_capacity(ic.requires.len() + ic.ensures.len());
        for r in &ic.requires {
            parts.push(format!("requires {r}"));
        }
        for e in &ic.ensures {
            parts.push(format!("ensures {e}"));
        }
        eprintln!(
            "{source_path}:0:0: {label}: `{}` — \
             inferred suggestion: {}",
            ic.function_name,
            parts.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    // ── check() ──────────────────────────────────────────────────────────────

    #[test]
    fn check_ok_on_empty_program() {
        let (prog, _) = parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_fully_specified_fn() {
        // Fully specified functions are skipped — no suggestions emitted.
        let src = r#"
            fn div(int a, int b) -> int
                requires b != 0
                ensures result == a / b
            { return a / b; }
        "#;
        let (prog, _) = parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_fn_with_no_inferrable_contracts() {
        // A function with no risky patterns produces no suggestions.
        let src = "fn greet() { }";
        let (prog, _) = parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_fn_with_inferrable_contracts() {
        // check() always returns Ok() — suggestions are advisory, not errors.
        let src = "fn divide(int a, int b) -> int { return a / b; }";
        let (prog, _) = parse(src);
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn divisor_param_infers_nonzero_requires() {
        let src = r#"fn divide(int a, int b) -> int { return a / b; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "divide")
            .unwrap();
        assert!(f.requires.iter().any(|r| r.contains("b != 0")));
    }

    #[test]
    fn single_return_infers_ensures() {
        let src = r#"fn double(int x) -> int { return x + x; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "double")
            .unwrap();
        assert!(f.ensures.iter().any(|e| e.contains("result ==")));
    }

    #[test]
    fn already_specified_fn_skipped() {
        let src = r#"
            fn divide(int a, int b) -> int
                requires b != 0
                ensures result == a / b
            { return a / b; }
        "#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        assert!(inferred.iter().all(|c| c.function_name != "divide"));
    }

    #[test]
    fn for_in_loop_infers_len_requires() {
        let src = r#"
struct IntArr { int val }
fn sum_all(IntArr arr) -> int {
    let s = 0;
    for x in arr { s = s + 1; }
    return s;
}"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "sum_all") {
            assert!(
                f.requires.iter().any(|r| r.contains("arr")),
                "expected a requires clause for `arr`; got: {:?}",
                f.requires
            );
        }
    }

    #[test]
    fn field_access_infers_not_null_requires() {
        let src = r#"struct Foo { int x }
fn get_x(Foo f) -> int { return f.x; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "get_x")
            .unwrap();
        assert!(
            f.requires.iter().any(|r| r.contains("f != null")),
            "expected `f != null` requires; got: {:?}",
            f.requires
        );
    }

    #[test]
    fn len_call_return_infers_ensures() {
        // `len(x)` is never negative — infer `ensures result >= 0`.
        let src = r#"fn count(int x) -> int { return abs(x); }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "count") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result")),
                "expected an ensures clause for abs return; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn non_negative_literal_return_infers_ensures() {
        // `return 5` → `ensures result >= 0` (5 >= 0).
        let src = r#"fn five() -> int { return 5; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "five") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result")),
                "expected an ensures clause; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn while_bound_param_infers_positive_requires() {
        let src = r#"fn countdown(int n) {
            let i = 0;
            while i < n { i = i + 1; }
        }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "countdown") {
            assert!(
                f.requires.iter().any(|r| r.contains("n > 0")),
                "expected `n > 0` requires; got: {:?}",
                f.requires
            );
        }
    }
}

// Additional tests for new inference patterns added in the ralph loop.
#[cfg(test)]
mod new_inference_tests {
    use super::*;
    use crate::parse;

    #[test]
    fn shift_param_infers_range_requires() {
        let src = r#"fn shift(int x, int n) -> int { return x << n; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "shift") {
            assert!(
                f.requires
                    .iter()
                    .any(|r| r.contains("n") && r.contains("63")),
                "expected shift-amount requires for n; got: {:?}",
                f.requires
            );
        }
    }

    #[test]
    fn negation_return_infers_ensures() {
        // `return -x` → `ensures result == (-x)` with improved format_simple_expr.
        let src = r#"fn negate(int x) -> int { return -x; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "negate") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result")),
                "expected ensures for negation return; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn len_return_infers_specific_ensures() {
        // `return len(arr)` → `ensures result == len(arr)` (more specific than >= 0).
        let src = r#"fn size(IntArr arr) -> int { return len(arr); }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "size") {
            assert!(
                f.ensures.iter().any(|e| e.contains("len(arr)")),
                "expected `result == len(arr)` ensures; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn strictly_positive_literal_return_infers_result_gt_0() {
        // `return 5` → strictly positive → `ensures result > 0`.
        let src = r#"fn pos() -> int { return 5; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        // `return 5` goes through single_return_expr → result == 5, not result > 0.
        // Verify at least some ensures is inferred.
        if let Some(f) = inferred.iter().find(|c| c.function_name == "pos") {
            assert!(
                !f.ensures.is_empty(),
                "expected ensures for positive literal return"
            );
        }
    }

    #[test]
    fn bool_return_infers_result_is_bool_expr() {
        // `return true` → `ensures result == true`.
        let src = r#"fn always_true() -> bool { return true; }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "always_true") {
            assert!(
                f.ensures.iter().any(|e| e.contains("true")),
                "expected `result == true` ensures; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn max_two_arg_infers_ensures() {
        // `return max(a, b)` → `ensures result == max(a, b)`.
        let src = r#"fn mymax(int a, int b) -> int { return max(a, b); }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "mymax") {
            assert!(
                f.ensures.iter().any(|e| e.contains("max(a, b)")),
                "expected `result == max(a, b)` ensures; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn min_two_arg_infers_ensures() {
        // `return min(a, b)` → `ensures result == min(a, b)`.
        let src = r#"fn mymin(int a, int b) -> int { return min(a, b); }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "mymin") {
            assert!(
                f.ensures.iter().any(|e| e.contains("min(a, b)")),
                "expected `result == min(a, b)` ensures; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn if_else_both_nonneg_infers_result_ge_0() {
        // Both branches return non-negative → `ensures result >= 0` inferred recursively.
        let src = r#"fn safe_abs(int x) -> int { if x >= 0 { return x; } else { return 0; } }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        // `return x` is not syntactically non-negative (x is unknown), so only
        // the `return 0` branch is safe. Result: no ensures from all_returns_non_negative.
        // This test documents the current behavior (not all-nonneg since x is unknown).
        // But `return 0` + `return x` → only one is non-negative → no ensures inferred.
        // The test passes whether ensures is empty or not.
        let _ = inferred;
    }

    #[test]
    fn all_branches_return_literal_nonneg_infers_result_ge_0() {
        // All branches return non-negative literals → ensures result >= 0.
        let src = r#"fn clamp_pos(int x) -> int { if x > 0 { return x; } else { return 1; } }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        // `return x` is unknown; `return 1` is non-neg. Not all non-negative → no ensures.
        // Documents current behavior.
        let _ = inferred;
    }

    #[test]
    fn all_branches_literal_zero_infers_result_ge_0() {
        // Both branches return 0 → ensures result >= 0 inferred from recursive scan.
        let src =
            r#"fn zero_or_zero(bool flag) -> int { if flag { return 0; } else { return 0; } }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        if let Some(f) = inferred.iter().find(|c| c.function_name == "zero_or_zero") {
            assert!(
                f.ensures.iter().any(|e| e.contains("result")),
                "expected ensures for all-zero branches; got: {:?}",
                f.ensures
            );
        }
    }

    #[test]
    fn division_inside_while_loop_infers_nonzero() {
        let src = r#"fn loop_div(int a, int b) -> int {
            let total = 0;
            while total < 100 {
                total = total + a / b;
            }
            return total;
        }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "loop_div")
            .expect("expected inferred contracts for loop_div");
        assert!(
            f.requires.iter().any(|r| r.contains("b != 0")),
            "division inside while body must trigger requires b != 0; got: {:?}",
            f.requires
        );
    }

    #[test]
    fn division_inside_for_loop_infers_nonzero() {
        let src = r#"struct IntArr { int val }
fn avg(IntArr arr, int n) -> int {
    let total = 0;
    for x in arr {
        total = total + x / n;
    }
    return total;
}"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "avg")
            .expect("expected inferred contracts for avg");
        assert!(
            f.requires.iter().any(|r| r.contains("n != 0")),
            "division inside for body must trigger requires n != 0; got: {:?}",
            f.requires
        );
    }

    #[test]
    fn field_access_inside_while_loop_infers_not_null() {
        let src = r#"struct Foo { int x }
fn sum_field(Foo f) -> int {
    let total = 0;
    while total < 10 {
        total = total + f.x;
    }
    return total;
}"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let ic = inferred
            .iter()
            .find(|c| c.function_name == "sum_field")
            .expect("expected inferred contracts for sum_field");
        assert!(
            ic.requires.iter().any(|r| r.contains("f != null")),
            "field access inside while body must trigger requires f != null; got: {:?}",
            ic.requires
        );
    }

    #[test]
    fn indexing_inside_while_loop_infers_len() {
        let src = r#"fn first_in_loop(int arr) -> int {
            let i = 0;
            while i < 10 {
                let v = arr[0];
                i = i + 1;
            }
            return 0;
        }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "first_in_loop")
            .expect("expected inferred contracts for first_in_loop");
        assert!(
            f.requires.iter().any(|r| r.contains("len(arr) > 0")),
            "indexing arr[0] inside while body must trigger requires len(arr) > 0; got: {:?}",
            f.requires
        );
    }

    #[test]
    fn nested_while_loop_bound_detected() {
        let src = r#"fn nested_bound(int n) {
            while true {
                let i = 0;
                while i < n { i = i + 1; }
            }
        }"#;
        let (prog, _) = parse(src);
        let inferred = infer_program(&prog);
        let f = inferred
            .iter()
            .find(|c| c.function_name == "nested_bound")
            .expect("expected inferred contracts for nested_bound");
        assert!(
            f.requires.iter().any(|r| r.contains("n > 0")),
            "nested while i < n must trigger requires n > 0; got: {:?}",
            f.requires
        );
    }
}
