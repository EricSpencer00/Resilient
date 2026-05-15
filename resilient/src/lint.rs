//! RES-198: `resilient lint` — five starter lints.
//!
//! Each lint has a stable code (`L0001`..`L0005`) and a
//! `# [allow]`-style suppress syntax: `// resilient: allow L0003`
//! on the line IMMEDIATELY ABOVE the offending node.
//!
//! Lints are WARNINGS by default. The CLI's `--deny L0001`
//! (mirrors `rustc -D`) escalates a specific code to error
//! severity; `--allow L0001` downgrades to suppressed. Unknown
//! codes on either flag are a usage error.
//!
//! Design notes
//! ============
//! - We build on the existing AST + span machinery (no new
//!   lexer work). Comment-based suppress is reconstructed by
//!   scanning the source text for the allow pattern independently
//!   of the parser; the set of suppressed `(line, code)` pairs is
//!   the filter applied to the raw lint output.
//! - Lints walk the AST top-down. Each lint is a separate
//!   function so a future `--only L0003` or `-W all` escalation
//!   has a clean seam to hook into.
//! - The module exports `check(program, source) -> Vec<Lint>`.
//!   Main wires that into the `lint <file>` subcommand.

use crate::{Node, Pattern, span::Span};
use std::sync::atomic::{AtomicBool, Ordering};

/// RES-198: one lint hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lint {
    /// Stable code, e.g. "L0001". Matches the code a user
    /// writes in `// resilient: allow L0001` to suppress.
    pub code: String,
    pub severity: Severity,
    /// Human-friendly diagnostic text.
    pub message: String,
    /// Location of the offending node (1-indexed).
    pub line: u32,
    pub column: u32,
}

/// RES-198: lint severity. Warning by default; `--deny` on the
/// CLI escalates to Error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Warning => write!(f, "warning"),
            Severity::Error => write!(f, "error"),
        }
    }
}

/// RES-198: the stable list of lint codes this module emits.
/// `--deny <code>` / `--allow <code>` arguments are validated
/// against this list in the CLI so typos are rejected early.
pub const KNOWN_CODES: &[&str] = &[
    "L0001", // unused local binding
    "L0002", // unreachable arm after `_`
    "L0003", // self-comparison `x == x`
    "L0004", // mixing `&&` and `||` without parens
    "L0005", // redundant trailing bare `return;`
    "L0006", // assume(false) vacuously discharges all verification obligations
    "L0007", // unreachable code after unconditional `return`
    "L0008", // duplicate identical struct literal match arm
    "L0009", // integer division by zero (literal / SMT-proven-possible)
    "L0010", // function has no requires/ensures contract
    "L0011", // RES-308: unused variable warning (let binding never read)
    "L0012", // RES-397: spec annotation lacks `// source:` provenance comment
    "L0013", // RES-798: unchecked array indexing (not proven in-bounds)
    "L0014", // function defined but never called (dead function)
    "L0015", // constant arithmetic expression overflows `int`
    "L0016", // constant boolean condition in `if` (always-true or always-false)
    "L0017", // variable binding shadows an outer binding of the same name
    "L0018", // function with return type may not return on all paths
    "L0019", // format() argument count does not match template placeholder count
    "L0020", // function parameter is never used in the body
    "L0021", // redundant boolean sub-expression (x && x, x || x)
    "L0022", // else branch after unconditional return is redundant
    "L0023", // tautological comparison with boolean literal (x == true)
    "L0024", // struct literal is missing one or more required fields
    "L0025", // unreachable code after infinite `while true` loop (no break)
    "L0026", // duplicate literal key in map literal (earlier binding shadowed)
    "L0027", // empty catch block silently discards the error
    "L0028", // negation of boolean literal (`!true` / `!false`) — use the literal directly
    "L0029", // comparison result discarded as statement — likely a typo for `=` or missed `assert`
    "L0030", // float equality comparison (`==` / `!=`) — almost always a bug; use an epsilon check
    "L0031", // double negation `!!x` is redundant — simplify to `x`
];

/// RES-778: process-wide policy switch for safety-critical CLI mode.
///
/// When enabled, `assume(false)` (L0006) is promoted from a warning to
/// a hard error and cannot be silenced by a local allow-comment.
static SAFETY_CRITICAL_MODE: AtomicBool = AtomicBool::new(false);

/// Enable or disable safety-critical lint policy for the current
/// process. Mirrors the atomic flag pattern already used by other
/// strict CLI modes in the compiler driver.
pub fn set_safety_critical_mode(on: bool) {
    SAFETY_CRITICAL_MODE.store(on, Ordering::Relaxed);
}

/// Returns true when safety-critical lint policy is active.
pub fn safety_critical_mode() -> bool {
    SAFETY_CRITICAL_MODE.load(Ordering::Relaxed)
}

/// RES-1376: cached trigger-presence flags for the lint passes.
/// Built by `scan_lint_triggers` in one AST visit; each lint pass is
/// gated on the flag for its trigger node so passes whose trigger
/// never appears in the program don't pay for a full AST walk.
#[derive(Default)]
struct LintTriggers {
    has_assume: bool,
    has_index: bool,
    has_match: bool,
    has_division: bool,
    has_infix: bool,
    has_function: bool,
    has_let: bool,
    has_block: bool,
    has_call: bool,
    has_integer_literal: bool,
    has_if_statement: bool,
    has_let_in_nested_block: bool,
    has_format_call: bool,
    has_if_with_else: bool,
    has_bool_literal: bool,
    has_while_true: bool,
    has_struct_literal: bool,
    has_map_literal: bool,
    has_try_catch: bool,
    has_prefix_expr: bool,
    has_float_literal: bool,
    has_expr_stmt_cmp: bool,
}

fn scan_lint_triggers(program: &Node) -> LintTriggers {
    let mut t = LintTriggers::default();
    scan_node(program, &mut t);
    t
}

fn scan_node(node: &Node, t: &mut LintTriggers) {
    match node {
        Node::Assume { .. } => t.has_assume = true,
        Node::IndexExpression { .. } => t.has_index = true,
        Node::Match { .. } => t.has_match = true,
        Node::InfixExpression { operator, .. } => {
            t.has_infix = true;
            if operator == "/" || operator == "%" {
                t.has_division = true;
            }
        }
        Node::Function { .. } => t.has_function = true,
        Node::LetStatement { .. } => t.has_let = true,
        Node::Block { stmts, .. } => {
            t.has_block = true;
            // L0017 trigger: a let inside a block (potential shadowing site).
            if stmts.iter().any(|s| matches!(s, Node::LetStatement { .. })) {
                t.has_let_in_nested_block = true;
            }
        }
        Node::CallExpression { function, .. } => {
            t.has_call = true;
            if matches!(function.as_ref(), Node::Identifier { name, .. } if name == "format") {
                t.has_format_call = true;
            }
        }
        Node::IntegerLiteral { .. } => t.has_integer_literal = true,
        Node::BooleanLiteral { .. } => t.has_bool_literal = true,
        Node::WhileStatement { condition, .. } => {
            if matches!(condition.as_ref(), Node::BooleanLiteral { value: true, .. }) {
                t.has_while_true = true;
            }
        }
        Node::IfStatement { alternative, .. } => {
            t.has_if_statement = true;
            if alternative.is_some() {
                t.has_if_with_else = true;
            }
        }
        Node::StructLiteral { .. } => t.has_struct_literal = true,
        Node::MapLiteral { .. } => t.has_map_literal = true,
        Node::TryCatch { .. } => t.has_try_catch = true,
        Node::PrefixExpression { .. } => t.has_prefix_expr = true,
        Node::FloatLiteral { .. } => t.has_float_literal = true,
        Node::ExpressionStatement { expr, .. } => {
            if matches!(expr.as_ref(), Node::InfixExpression { operator, .. }
                if matches!(operator.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">="))
            {
                t.has_expr_stmt_cmp = true;
            }
        }
        _ => {}
    }
    recurse_children(node, &mut |child| scan_node(child, t));
}

/// RES-198: top-level entry. Runs every lint, filters via the
/// `// resilient: allow LXXXX` comments found in `source`, and
/// returns the surviving diagnostics sorted by (line, column).
///
/// RES-1376: a single `scan_lint_triggers` AST visit caches which
/// lint trigger nodes appear; each `run_l00XX` is gated on the
/// matching flag. Lints whose trigger never appears are skipped —
/// single visit replaces up to 13 separate walks.
pub fn check(program: &Node, source: &str) -> Vec<Lint> {
    let mut out = Vec::new();
    let t = scan_lint_triggers(program);
    if t.has_function {
        run_l0001_unused_local(program, &mut out);
    }
    if t.has_match {
        run_l0002_unreachable_arm(program, &mut out);
    }
    if t.has_infix {
        run_l0003_self_comparison(program, &mut out);
        run_l0004_mixed_and_or(program, &mut out);
    }
    if t.has_function {
        run_l0005_redundant_return(program, &mut out);
    }
    if t.has_assume {
        run_l0006_assume_false(program, &mut out);
    }
    if t.has_block {
        run_l0007_unreachable_code(program, &mut out);
    }
    if t.has_match {
        run_l0008_duplicate_struct_match_arm(program, &mut out);
    }
    if t.has_division {
        run_l0009_division_by_zero(program, &mut out);
    }
    if t.has_function {
        run_l0010_no_contract(program, &mut out);
    }
    if t.has_let {
        run_l0011_unused_variable(program, &mut out);
    }
    if t.has_function {
        run_l0012_spec_provenance(program, source, &mut out);
    }
    if t.has_index {
        run_l0013_unchecked_indexing(program, &mut out);
    }
    if t.has_function {
        run_l0014_unused_function(program, &mut out);
    }
    if t.has_infix && t.has_integer_literal {
        run_l0015_const_overflow(program, &mut out);
    }
    if t.has_if_statement {
        run_l0016_constant_condition(program, &mut out);
    }
    if t.has_function && t.has_let_in_nested_block {
        run_l0017_variable_shadowing(program, &mut out);
    }
    if t.has_function {
        run_l0018_missing_return(program, &mut out);
    }
    if t.has_format_call {
        run_l0019_format_arity(program, &mut out);
    }
    if t.has_function {
        run_l0020_unused_parameter(program, &mut out);
    }
    if t.has_infix {
        run_l0021_redundant_bool_subexpr(program, &mut out);
    }
    if t.has_if_with_else {
        run_l0022_needless_else(program, &mut out);
    }
    if t.has_bool_literal && t.has_infix {
        run_l0023_bool_literal_comparison(program, &mut out);
    }
    if t.has_while_true && t.has_block {
        run_l0025_unreachable_after_infinite_loop(program, &mut out);
    }
    if t.has_struct_literal {
        run_l0024_struct_missing_fields(program, &mut out);
    }
    if t.has_map_literal {
        run_l0026_duplicate_map_key(program, &mut out);
    }
    if t.has_try_catch {
        run_l0027_empty_catch_block(program, &mut out);
    }
    if t.has_prefix_expr && t.has_bool_literal {
        run_l0028_negation_of_literal(program, &mut out);
    }
    if t.has_expr_stmt_cmp {
        run_l0029_comparison_result_discarded(program, &mut out);
    }
    if t.has_float_literal && t.has_infix {
        run_l0030_float_equality(program, &mut out);
    }
    if t.has_prefix_expr {
        run_l0031_double_negation(program, &mut out);
    }
    let safety_critical = safety_critical_mode();
    if safety_critical {
        for lint in out.iter_mut() {
            if lint.code == "L0006" {
                lint.severity = Severity::Error;
            }
        }
    }

    // Filter via allow-comments.
    //
    // RES-308: L0011 is the rustc-style sibling of L0001's
    // unused-let case (both fire on the same `let unused = ...`
    // pattern, but with different phrasings). Authors who write
    // `// resilient: allow L0001` above a let are saying "I know
    // this is unused" — the same intent should silence L0011.
    // Treat the L0001 allow as implying the L0011 allow for the
    // same line, so dual emission stays user-suppressible with
    // a single comment.
    //
    // RES-1515: skip the per-lint clone+contains pair entirely when
    // no `// resilient: allow ...` comments exist in the source.
    // The common case for every fixture in `examples/` and every
    // CI input is an empty `allows` set; the retain closure was
    // cloning `l.code` per lint just to ask a HashSet that was
    // guaranteed empty. The `safety_critical && l.code == "L0006"`
    // gate is also a no-op when nothing would otherwise drop the
    // lint — early-out keeps every lint in that case too.
    let allows = collect_allow_comments(source);
    if !allows.is_empty() {
        out.retain(|l| {
            if safety_critical && l.code == "L0006" {
                return true;
            }
            if allows.contains(&(l.line, l.code.clone())) {
                return false;
            }
            if l.code == "L0011" && allows.contains(&(l.line, "L0001".to_string())) {
                return false;
            }
            true
        });
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));
    out
}

/// RES-308: lint codes that are aliases of one another for
/// `--allow` / `--deny` purposes. `(primary, alias)` means a
/// flag targeting `primary` also affects `alias`. Today only
/// L0001 ↔ L0011 (unused-let warning re-phrased rustc-style).
pub const ALLOW_ALIASES: &[(&str, &str)] = &[("L0001", "L0011")];

/// RES-198: render a lint as a `<path>:<line>:<col>: <severity>[<code>]: <msg>`
/// single-line diagnostic. Matches the RES-080 prefix convention
/// used by the typechecker so users can copy-paste locations.
pub fn format_lint(l: &Lint, path: &str) -> String {
    format!(
        "{}:{}:{}: {}[{}]: {}",
        path, l.line, l.column, l.severity, l.code, l.message
    )
}

// ============================================================
// L0001: unused local binding
// ============================================================
//
// For each top-level fn, collect `let` + `static let` bindings
// inside the body, then check whether each bound name is
// referenced anywhere else in the body. Names starting with `_`
// are skipped (convention: user explicitly marks the binding as
// intentional).
//
// Limitation: shadowing isn't tracked precisely. `let x = 1;
// let x = 2;` counts `x` as "used" once the second binding's
// body or a later statement references `x`. MVP; precise
// shadow-aware analysis is a follow-up.

fn run_l0001_unused_local(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0001_check_body(body, out);
            }
            // RES-239: descend into impl block methods.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0001_check_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0001_check_body(body: &Node, out: &mut Vec<Lint>) {
    // RES-1533: borrow let-binding names and identifier-read names
    // from the AST into the `lets` Vec and `used` HashSet rather
    // than cloning every name. Same pattern as RES-1500 / RES-1525.
    let mut lets: Vec<(&str, Span)> = Vec::new();
    collect_lets_in(body, &mut lets);
    if !lets.is_empty() {
        let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
        collect_identifier_reads_in(body, &mut used);
        for (name, span) in &lets {
            if name.starts_with('_') {
                continue;
            }
            if !used.contains(*name) {
                out.push(Lint {
                    code: "L0001".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "unused local binding `{}` — prefix with `_` to silence",
                        name
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    // RES-259: check match-arm pattern bindings (scoped per arm).
    // This is always called, regardless of whether `let` bindings exist.
    l0001_check_match_arms(body, out);
}

/// RES-259: collect the names bound by a pattern (one level of binding
/// per pattern, recursing into `Or` first-branch and `Bind` inner).
fn collect_pattern_bindings(pattern: &Pattern) -> Vec<&str> {
    match pattern {
        Pattern::Identifier(name) => vec![name.as_str()],
        Pattern::Bind(name, inner) => {
            let mut names = vec![name.as_str()];
            names.extend(collect_pattern_bindings(inner));
            names
        }
        // Or-patterns: all branches bind the same names (parser invariant);
        // read the first branch only to avoid duplicates.
        Pattern::Or(branches) => {
            if let Some(first) = branches.first() {
                collect_pattern_bindings(first)
            } else {
                vec![]
            }
        }
        // Wildcard and Literal introduce no bindings.
        Pattern::Wildcard | Pattern::Literal(_) => vec![],
        Pattern::Struct { fields, .. } => {
            // Pre-size to fields.len(): each field's sub-pattern most
            // commonly binds 0–1 names (Identifier / Wildcard), so the
            // field count is a tight upper bound for the typical case.
            // Sub-patterns that bind more (nested destructure) trigger
            // extend's amortised growth from there.
            let mut names = Vec::with_capacity(fields.len());
            for (_, sub) in fields {
                names.extend(collect_pattern_bindings(sub.as_ref()));
            }
            names
        }
        // RES-375: `Some(inner)` forwards to inner; `None` has no bindings.
        Pattern::Some(inner) => collect_pattern_bindings(inner.as_ref()),
        Pattern::None => vec![],
        // RES-923: Result patterns mirror Option's behaviour.
        Pattern::Ok(inner) | Pattern::Err(inner) => collect_pattern_bindings(inner.as_ref()),
        // RES-915: range patterns bind no names.
        Pattern::Range { .. } => vec![],
        // RES-400: enum-variant pattern bindings.
        Pattern::EnumVariant { payload, .. } => match payload {
            crate::EnumPatternPayload::None => vec![],
            crate::EnumPatternPayload::Named(fields) => {
                let mut names = Vec::with_capacity(fields.len());
                for (_, sub) in fields {
                    names.extend(collect_pattern_bindings(sub.as_ref()));
                }
                names
            }
            crate::EnumPatternPayload::Tuple(subs) => {
                let mut names = Vec::with_capacity(subs.len());
                for sub in subs {
                    names.extend(collect_pattern_bindings(sub));
                }
                names
            }
        },
        // RES-931: tuple-struct destructure — recurse into each field pattern.
        Pattern::TupleStruct { fields, .. } => {
            let mut names = Vec::with_capacity(fields.len());
            for sub in fields {
                names.extend(collect_pattern_bindings(sub));
            }
            names
        }
        // RES-932: anonymous tuple destructure — recurse positionally.
        Pattern::Tuple(items) => {
            let mut names = Vec::with_capacity(items.len());
            for sub in items {
                names.extend(collect_pattern_bindings(sub));
            }
            names
        }
    }
}

/// RES-259: walk every `Node::Match` in `node` and, for each arm,
/// check whether the arm's pattern bindings are used within that
/// arm's guard and body. Reports L0001 for each unused binding.
fn l0001_check_match_arms(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Match {
            scrutinee, arms, ..
        } => {
            // Recurse into the scrutinee first.
            l0001_check_match_arms(scrutinee, out);

            for (pattern, guard, arm_body) in arms {
                let bindings = collect_pattern_bindings(pattern);
                if !bindings.is_empty() {
                    // Collect reads from the guard (if any) and the arm body.
                    let mut used: std::collections::HashSet<&str> =
                        std::collections::HashSet::new();
                    if let Some(g) = guard {
                        collect_identifier_reads_in(g, &mut used);
                    }
                    collect_identifier_reads_in(arm_body, &mut used);

                    // Use the arm body's span for the diagnostic position.
                    let (line, col) = span_of(arm_body)
                        .map(|s| (s.start.line as u32, s.start.column as u32))
                        .unwrap_or((1, 1));

                    for name in &bindings {
                        if name.starts_with('_') {
                            continue;
                        }
                        if !used.contains(*name) {
                            out.push(Lint {
                                code: "L0001".into(),
                                severity: Severity::Warning,
                                message: format!(
                                    "unused local binding `{}` — prefix with `_` to silence",
                                    name
                                ),
                                line,
                                column: col,
                            });
                        }
                    }
                }

                // Recurse into nested match expressions inside the arm body.
                l0001_check_match_arms(arm_body, out);
                if let Some(g) = guard {
                    l0001_check_match_arms(g, out);
                }
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0001_check_match_arms(s, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            l0001_check_match_arms(value, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0001_check_match_arms(condition, out);
            l0001_check_match_arms(consequence, out);
            if let Some(a) = alternative {
                l0001_check_match_arms(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0001_check_match_arms(condition, out);
            l0001_check_match_arms(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0001_check_match_arms(iterable, out);
            l0001_check_match_arms(body, out);
        }
        Node::LiveBlock { body, .. } => l0001_check_match_arms(body, out),
        Node::ReturnStatement { value: Some(v), .. } => l0001_check_match_arms(v, out),
        Node::ExpressionStatement { expr, .. } => l0001_check_match_arms(expr, out),
        _ => {}
    }
}

fn collect_lets_in<'a>(node: &'a Node, out: &mut Vec<(&'a str, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            collect_lets_in(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            collect_lets_in(value, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_lets_in(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_lets_in(condition, out);
            collect_lets_in(consequence, out);
            if let Some(a) = alternative {
                collect_lets_in(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_lets_in(condition, out);
            collect_lets_in(body, out);
        }
        Node::ForInStatement {
            name,
            iterable,
            body,
            span,
            ..
        } => {
            if !name.starts_with('_') {
                out.push((name.as_str(), *span));
            }
            collect_lets_in(iterable, out);
            collect_lets_in(body, out);
        }
        Node::LiveBlock { body, .. } => collect_lets_in(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_lets_in(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_lets_in(g, out);
                }
                collect_lets_in(arm_body, out);
            }
        }
        // RES-237: struct destructure — each local binding name is a
        // new `let`-equivalent that L0001 should track.
        Node::LetDestructureStruct {
            fields,
            value,
            span,
            ..
        } => {
            for (_field_name, local_name) in fields {
                out.push((local_name.as_str(), *span));
            }
            collect_lets_in(value, out);
        }
        _ => {}
    }
}

fn collect_identifier_reads_in<'a>(node: &'a Node, out: &mut std::collections::HashSet<&'a str>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.as_str());
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => {
            collect_identifier_reads_in(v, out);
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::Assignment { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            collect_identifier_reads_in(expr, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            collect_identifier_reads_in(condition, out);
            collect_identifier_reads_in(consequence, out);
            if let Some(a) = alternative {
                collect_identifier_reads_in(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            collect_identifier_reads_in(condition, out);
            collect_identifier_reads_in(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            collect_identifier_reads_in(iterable, out);
            collect_identifier_reads_in(body, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                collect_identifier_reads_in(s, out);
            }
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            collect_identifier_reads_in(body, out);
            for inv in invariants {
                collect_identifier_reads_in(inv, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            collect_identifier_reads_in(left, out);
            collect_identifier_reads_in(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            collect_identifier_reads_in(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            collect_identifier_reads_in(function, out);
            for a in arguments {
                collect_identifier_reads_in(a, out);
            }
        }
        Node::TryExpression { expr, .. } => {
            collect_identifier_reads_in(expr, out);
        }
        Node::OptionalChain { object, access, .. } => {
            collect_identifier_reads_in(object, out);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    collect_identifier_reads_in(a, out);
                }
            }
        }
        Node::IndexExpression { target, index, .. } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(index, out);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(index, out);
            collect_identifier_reads_in(value, out);
        }
        Node::FieldAccess { target, .. } => {
            collect_identifier_reads_in(target, out);
        }
        Node::FieldAssignment { target, value, .. } => {
            collect_identifier_reads_in(target, out);
            collect_identifier_reads_in(value, out);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                collect_identifier_reads_in(i, out);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                collect_identifier_reads_in(v, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            collect_identifier_reads_in(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    collect_identifier_reads_in(g, out);
                }
                collect_identifier_reads_in(arm_body, out);
            }
        }
        Node::Assert { condition, .. } => {
            collect_identifier_reads_in(condition, out);
        }
        // RES-237: assume(cond[, msg]) — identifiers inside the condition
        // and optional message are reads.
        Node::Assume {
            condition, message, ..
        } => {
            collect_identifier_reads_in(condition, out);
            if let Some(msg) = message {
                collect_identifier_reads_in(msg, out);
            }
        }
        // RES-237: {k -> v, ...} map literal — both keys and values are reads.
        Node::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                collect_identifier_reads_in(k, out);
                collect_identifier_reads_in(v, out);
            }
        }
        // RES-237: #{item, ...} set literal — each item is a read.
        Node::SetLiteral { items, .. } => {
            for item in items {
                collect_identifier_reads_in(item, out);
            }
        }
        // RES-237: struct destructure — the RHS value is a read.
        Node::LetDestructureStruct { value, .. } => {
            collect_identifier_reads_in(value, out);
        }
        _ => {}
    }
}

// ============================================================
// L0002: unreachable arm after `_ =>`
// ============================================================
//
// A `_` pattern matches anything, so any arm textually following
// it can never fire. Walk every Match node; once a wildcard-only
// arm appears, flag the start of every subsequent arm.
//
// A `_` nested inside a `Pattern::Or` branch doesn't itself
// render the rest of the match unreachable (each branch of the
// Or tests independently); only a top-level wildcard arm does.
//
// RES-232: `Pattern::Bind` whose inner pattern is a default (e.g.
// `n @ _`, `n @ m`) also catches every value — treat as catch-all.

/// RES-232: mirrors `typechecker::pattern_is_default`. Returns `true`
/// when the pattern matches every value (wildcard, bare identifier,
/// bind whose inner is default, or-pattern with at least one default
/// branch).
fn pattern_is_default_for_lint(p: &Pattern) -> bool {
    match p {
        Pattern::Wildcard | Pattern::Identifier(_) => true,
        // RES-915: range patterns never catch every Int (e.g. `1..=5`
        // misses 0, 6, …).
        Pattern::Literal(_) | Pattern::Range { .. } => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default_for_lint),
        Pattern::Bind(_, inner) => pattern_is_default_for_lint(inner),
        Pattern::Struct { fields, .. } => fields
            .iter()
            .all(|(_, sub)| pattern_is_default_for_lint(sub.as_ref())),
        // RES-375: Option patterns are never catch-alls by themselves.
        Pattern::Some(_) | Pattern::None | Pattern::Ok(_) | Pattern::Err(_) => false,
        // RES-400: enum-variant patterns are never catch-alls — each
        // matches one specific variant.
        Pattern::EnumVariant { .. } => false,
        // RES-931: a tuple-struct pattern is a catch-all iff every
        // positional sub-pattern is itself a default — `Pair(_, _)`
        // catches every `Pair`, but `Pair(0, _)` does not.
        Pattern::TupleStruct { fields, .. } => fields.iter().all(pattern_is_default_for_lint),
        // RES-932: same shape — `(_, _)` is a catch-all over 2-tuples;
        // `(0, _)` is not.
        Pattern::Tuple(items) => items.iter().all(pattern_is_default_for_lint),
    }
}

fn run_l0002_unreachable_arm(program: &Node, out: &mut Vec<Lint>) {
    walk_matches(program, out);
}

fn walk_matches(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_matches(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_matches(body, out),
        // RES-239: descend into impl block methods.
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_matches(method, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_matches(s, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            // Find the first arm whose pattern is a bare wildcard.
            // Report subsequent arms at the arm body's span (the
            // closest accessible position — `Pattern` itself
            // doesn't carry a span today). Falls back to the
            // scrutinee's span when the body has a default span.
            let scrut_line = match span_of(scrutinee) {
                Some(s) => s.start.line as u32,
                None => 1,
            };
            let scrut_col = match span_of(scrutinee) {
                Some(s) => s.start.column as u32,
                None => 1,
            };
            let mut saw_wild = false;
            for (pat, _guard, arm_body) in arms {
                if saw_wild {
                    let arm_span = span_of(arm_body);
                    let (line, col) = match arm_span {
                        Some(s) if s.start.line > 0 => (s.start.line as u32, s.start.column as u32),
                        _ => (scrut_line, scrut_col),
                    };
                    out.push(Lint {
                        code: "L0002".into(),
                        severity: Severity::Warning,
                        message:
                            "arm is unreachable — an earlier `_` arm already matches everything"
                                .into(),
                        line,
                        column: col,
                    });
                }
                walk_matches(arm_body, out);
                if pattern_is_default_for_lint(pat) {
                    saw_wild = true;
                }
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_matches(condition, out);
            walk_matches(consequence, out);
            if let Some(a) = alternative {
                walk_matches(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_matches(condition, out);
            walk_matches(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_matches(iterable, out);
            walk_matches(body, out);
        }
        Node::LiveBlock { body, .. } => walk_matches(body, out),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_matches(value, out);
        }
        Node::ExpressionStatement { expr, .. } => walk_matches(expr, out),
        Node::InfixExpression { left, right, .. } => {
            walk_matches(left, out);
            walk_matches(right, out);
        }
        Node::PrefixExpression { right, .. } => walk_matches(right, out),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_matches(function, out);
            for a in arguments {
                walk_matches(a, out);
            }
        }
        _ => {}
    }
}

fn struct_literal_match_arm_key(pat: &Pattern) -> Option<String> {
    let Pattern::Struct {
        struct_name,
        fields,
        has_rest,
    } = pat
    else {
        return None;
    };
    if *has_rest || fields.is_empty() {
        return None;
    }
    // RES-1774: pre-size to fields.len() — one push per field on the
    // happy path (loop returns None early on any non-literal).
    let mut parts = Vec::with_capacity(fields.len());
    for (fname, sub) in fields {
        match sub.as_ref() {
            Pattern::Literal(Node::IntegerLiteral { value, .. }) => {
                parts.push(format!("{}={}", fname, value));
            }
            _ => return None,
        }
    }
    parts.sort();
    Some(format!("{}|{}", struct_name, parts.join("|")))
}

fn run_l0008_duplicate_struct_match_arm(program: &Node, out: &mut Vec<Lint>) {
    walk_dup_struct_arms(program, out);
}

fn walk_dup_struct_arms(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                walk_dup_struct_arms(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_dup_struct_arms(body, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_dup_struct_arms(method, out);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                walk_dup_struct_arms(s, out);
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_dup_struct_arms(scrutinee, out);
            let scrut_line = match span_of(scrutinee) {
                Some(s) => s.start.line as u32,
                None => 1,
            };
            let scrut_col = match span_of(scrutinee) {
                Some(s) => s.start.column as u32,
                None => 1,
            };
            let mut seen = std::collections::HashSet::<String>::new();
            for (pat, guard, arm_body) in arms {
                if guard.is_none()
                    && let Some(k) = struct_literal_match_arm_key(pat)
                    && !seen.insert(k)
                {
                    let arm_span = span_of(arm_body);
                    let (line, col) = match arm_span {
                        Some(s) if s.start.line > 0 => (s.start.line as u32, s.start.column as u32),
                        _ => (scrut_line, scrut_col),
                    };
                    out.push(Lint {
                        code: "L0008".into(),
                        severity: Severity::Warning,
                        message: "unreachable match arm — an earlier arm matches the same struct literal pattern"
                            .into(),
                        line,
                        column: col,
                    });
                }
                walk_dup_struct_arms(arm_body, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_dup_struct_arms(condition, out);
            walk_dup_struct_arms(consequence, out);
            if let Some(a) = alternative {
                walk_dup_struct_arms(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_dup_struct_arms(condition, out);
            walk_dup_struct_arms(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_dup_struct_arms(iterable, out);
            walk_dup_struct_arms(body, out);
        }
        Node::LiveBlock { body, .. } => walk_dup_struct_arms(body, out),
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::ReturnStatement {
            value: Some(value), ..
        } => {
            walk_dup_struct_arms(value, out);
        }
        Node::ExpressionStatement { expr, .. } => walk_dup_struct_arms(expr, out),
        Node::InfixExpression { left, right, .. } => {
            walk_dup_struct_arms(left, out);
            walk_dup_struct_arms(right, out);
        }
        Node::PrefixExpression { right, .. } => walk_dup_struct_arms(right, out),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            walk_dup_struct_arms(function, out);
            for a in arguments {
                walk_dup_struct_arms(a, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0003: comparison `x == x` always true
// ============================================================
//
// Walk every InfixExpression with operator `==` or `!=`. If
// both sides are syntactically the same Identifier, flag.
// `!=` gets flagged too: `x != x` is always false, equally
// suspect. We report both under the single L0003 code with
// wording tuned to the operator.

fn run_l0003_self_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_self_comparisons(program, out);
}

fn walk_self_comparisons(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "==" || operator == "!=")
        && let (Node::Identifier { name: ln, .. }, Node::Identifier { name: rn, .. }) =
            (left.as_ref(), right.as_ref())
        && ln == rn
    {
        let always = if operator == "==" {
            "always true"
        } else {
            "always false"
        };
        out.push(Lint {
            code: "L0003".into(),
            severity: Severity::Warning,
            message: format!("comparing `{}` to itself is {} (likely a typo)", ln, always),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    // Recurse generically.
    recurse_children(node, &mut |child| walk_self_comparisons(child, out));
}

// ============================================================
// L0004: mixing `&&` and `||` without parens
// ============================================================
//
// Flag any InfixExpression whose operator is `&&` / `||` AND
// whose immediate child (left or right) has the opposite
// boolean operator. Paren-disambiguation isn't tracked in the
// AST, so this has a controlled false-positive rate on
// explicitly-parenthesized code — users suppress with
// `allow L0004`.

fn run_l0004_mixed_and_or(program: &Node, out: &mut Vec<Lint>) {
    walk_and_or(program, out);
}

fn walk_and_or(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
    {
        let opposite = match operator.as_str() {
            "&&" => Some("||"),
            "||" => Some("&&"),
            _ => None,
        };
        if let Some(opp) = opposite
            && (has_top_level_op(left, opp) || has_top_level_op(right, opp))
        {
            out.push(Lint {
                code: "L0004".into(),
                severity: Severity::Warning,
                message: format!(
                    "mixing `{}` and `{}` — add explicit parens to disambiguate precedence",
                    operator, opp
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_and_or(child, out));
}

fn has_top_level_op(node: &Node, op: &str) -> bool {
    matches!(node, Node::InfixExpression { operator, .. } if operator == op)
}

/// RES-198: best-effort span extraction. Mirrors the helper in
/// `lsp_server`; duplicated here so `lint` can stay feature-gate
/// independent of `lsp`.
fn span_of(node: &Node) -> Option<Span> {
    match node {
        Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::Identifier { span, .. }
        | Node::InfixExpression { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. }
        | Node::TryExpression { span, .. }
        | Node::OptionalChain { span, .. }
        | Node::ArrayLiteral { span, .. }
        | Node::IndexExpression { span, .. }
        | Node::FieldAccess { span, .. }
        | Node::StructLiteral { span, .. }
        | Node::Block { span, .. }
        | Node::Match { span, .. }
        | Node::LetStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Function { span, .. }
        | Node::LiveBlock { span, .. }
        | Node::Assert { span, .. }
        | Node::Assume { span, .. } => Some(*span),
        _ => None,
    }
}

// ============================================================
// L0005: redundant trailing `return;`
// ============================================================
//
// A bare `return;` (no value) at the end of a function body is
// redundant — the function would return Void without it. We
// don't flag `return VALUE;` trailing, since that IS load-
// bearing (Resilient doesn't have implicit-last-expression
// returns today).

fn run_l0005_redundant_return(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0005_check_fn_body(body, out);
            }
            // RES-239: check methods inside impl blocks.
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0005_check_fn_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0005_check_fn_body(body: &Node, out: &mut Vec<Lint>) {
    if let Node::Block {
        stmts: body_stmts, ..
    } = body
        && let Some(Node::ReturnStatement { value: None, span }) = body_stmts.last()
    {
        out.push(Lint {
            code: "L0005".into(),
            severity: Severity::Warning,
            message: "redundant `return;` at end of function body — remove it".into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
}

// ============================================================
// L0006: assume(false) — vacuously true hypothesis
// ============================================================
//
// `assume(false)` causes the SMT verifier to treat `false` as a
// precondition, making every subsequent obligation trivially satisfied
// (ex-falso). At runtime the call halts unconditionally. This is
// almost always a mistake; flag it as a warning.
//
// Only `assume(false)` with a literal `false` argument is flagged.
// `assume(true)` and `assume(x > 0)` are silent.

fn run_l0006_assume_false(program: &Node, out: &mut Vec<Lint>) {
    walk_assume_false(program, out);
}

fn walk_assume_false(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Assume {
        condition, span, ..
    } = node
        && matches!(
            condition.as_ref(),
            Node::BooleanLiteral { value: false, .. }
        )
    {
        out.push(Lint {
            code: "L0006".into(),
            severity: Severity::Warning,
            message: "assume(false): all subsequent verification obligations in this block \
                are vacuously discharged; code after this point is unreachable at runtime"
                .into(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_assume_false(child, out));
}

// ============================================================
// L0007: unreachable code after unconditional `return`
// ============================================================
//
// Walk every Block node. Once a `ReturnStatement` is seen, any
// subsequent node in the same block is unreachable. Only the
// FIRST unreachable statement is reported (pointing to it tells
// the user exactly where dead code begins). Nested blocks are
// walked independently — a `return` inside an `if` branch does
// not make statements after the `if` unreachable.
//
// The language does not yet have `break`/`continue` statements;
// if those are added, this lint should be extended to treat them
// as additional terminators.

fn run_l0007_unreachable_code(program: &Node, out: &mut Vec<Lint>) {
    walk_unreachable(program, out);
}

fn walk_unreachable(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::Block { stmts, .. } => {
            let mut saw_terminator = false;
            for stmt in stmts {
                if saw_terminator {
                    if let Some(span) = span_of(stmt) {
                        out.push(Lint {
                            code: "L0007".into(),
                            severity: Severity::Warning,
                            message: "unreachable code after `return`".into(),
                            line: span.start.line as u32,
                            column: span.start.column as u32,
                        });
                    }
                    // Report only the first unreachable statement.
                    break;
                }
                if matches!(stmt, Node::ReturnStatement { .. }) {
                    saw_terminator = true;
                }
                // Descend into nested blocks regardless of whether we have
                // seen a terminator — the nested scope is independent.
                walk_unreachable(stmt, out);
            }
        }
        Node::Program(stmts) => {
            for s in stmts {
                walk_unreachable(&s.node, out);
            }
        }
        Node::Function { body, .. } => walk_unreachable(body, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                walk_unreachable(method, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            walk_unreachable(condition, out);
            walk_unreachable(consequence, out);
            if let Some(a) = alternative {
                walk_unreachable(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            walk_unreachable(condition, out);
            walk_unreachable(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            walk_unreachable(iterable, out);
            walk_unreachable(body, out);
        }
        Node::LiveBlock { body, .. } => walk_unreachable(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            walk_unreachable(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    walk_unreachable(g, out);
                }
                walk_unreachable(arm_body, out);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => {
            walk_unreachable(value, out);
        }
        Node::ReturnStatement { value: Some(v), .. } => walk_unreachable(v, out),
        Node::ExpressionStatement { expr, .. } => walk_unreachable(expr, out),
        _ => {}
    }
}

// ============================================================
// L0009: integer division by zero (RES-350)
// ============================================================
//
// Division by zero on Cortex-M is a hard fault — no signal, no
// trap handler in the default configuration, just a locked-up
// core. This lint flags `a / b` and `a % b` when `b` cannot be
// proven non-zero given the information available.
//
// Two modes:
//
// - Default build: only literal-zero divisors fire. `a / 0`,
//   `a % 0`, `a / 0.0`, `a % 0.0` are statically obvious bugs
//   and deserve the warning regardless of SMT availability.
// - `--features z3`: the lint additionally asks Z3 "given the
//   enclosing fn's `requires` clauses, is `divisor != 0`
//   provable?". If Z3 returns `Some(true)`, the divisor is
//   proven non-zero and the lint stays silent. Any other verdict
//   (`Some(false)`, `None`, or timeout) triggers the warning with
//   a hint pointing at the missing precondition.
//
// The ticket proposed code `L0004` for this lint, but `L0004` is
// already shipped as the mixed-`&&`/`||` paren warning; renaming
// would silently flip the meaning of every `// resilient: allow
// L0004` comment in the wild. We allocate `L0009` — the next
// unused slot — and note the conflict in the PR that added this
// file.

fn run_l0009_division_by_zero(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, requires, .. } => {
                let axioms = combine_axioms(requires, body);
                l0009_check_body(body, &axioms, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, requires, .. } = method {
                        let axioms = combine_axioms(requires, body);
                        l0009_check_body(body, &axioms, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// RES-133b: combine the function's `requires` clauses with the
/// leading `assume(P)` predicates from the body. The result is the
/// axiom set the divide-by-zero prover sees — assumes at the start
/// of a fn body are valid axioms because they're runtime-checked
/// before any expression evaluates.
fn combine_axioms(requires: &[Node], body: &Node) -> Vec<Node> {
    let mut axioms: Vec<Node> = requires.to_vec();
    axioms.extend(crate::assume_axioms::collect_leading_assume_axioms(body));
    axioms
}

/// RES-350: walk one fn body, flagging divisions by zero. The
/// `requires` slice belongs to the enclosing fn and is handed to
/// Z3 as assumption axioms (feature-gated).
fn l0009_check_body(body: &Node, requires: &[Node], out: &mut Vec<Lint>) {
    walk_divisions(body, requires, out);
}

fn walk_divisions(node: &Node, requires: &[Node], out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "/" || operator == "%")
    {
        match right.as_ref() {
            Node::IntegerLiteral { value: 0, .. } => {
                out.push(Lint {
                    code: "L0009".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "division by zero: `{}` with a literal-zero divisor is a hard fault on Cortex-M",
                        operator
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            Node::FloatLiteral { value, .. } if *value == 0.0 => {
                out.push(Lint {
                    code: "L0009".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "division by zero: `{}` with a literal-zero divisor is a hard fault on Cortex-M",
                        operator
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            other => {
                // Non-literal divisor: under `--features z3` ask the
                // solver whether the enclosing fn's preconditions
                // force it non-zero. Without Z3 we stay silent to
                // avoid false positives.
                if let Some(lint) = l0009_z3_check(left, operator, other, requires, span) {
                    out.push(lint);
                }
            }
        }
    }
    // Recurse through the same generic walker the other lints use.
    recurse_children(node, &mut |child| walk_divisions(child, requires, out));
}

#[cfg(feature = "z3")]
fn l0009_z3_check(
    _left: &Node,
    operator: &str,
    right: &Node,
    requires: &[Node],
    span: &Span,
) -> Option<Lint> {
    use crate::verifier_z3;
    // Construct the synthetic obligation `<right> != 0`.
    let obligation = Node::InfixExpression {
        left: Box::new(right.clone()),
        operator: "!=".to_string(),
        right: Box::new(Node::IntegerLiteral {
            value: 0,
            span: crate::span::Span::default(),
        }),
        span: crate::span::Span::default(),
    };
    let empty = std::collections::HashMap::new();
    // 1 s is plenty for simple non-zero obligations; if the user
    // has unusually complex preconditions they can downgrade via
    // `// resilient: allow L0009`.
    let (verdict, _cert, _cx, _timeout) =
        verifier_z3::prove_with_axioms_and_timeout(&obligation, &empty, requires, 1000);
    if verdict == Some(true) {
        return None;
    }
    Some(Lint {
        code: "L0009".into(),
        severity: Severity::Warning,
        message: format!(
            "division may be by zero: `{}` divisor is not proven non-zero; \
             add `requires <divisor> != 0;` to the enclosing fn, or \
             silence with `// resilient: allow L0009`",
            operator
        ),
        line: span.start.line as u32,
        column: span.start.column as u32,
    })
}

#[cfg(not(feature = "z3"))]
fn l0009_z3_check(
    _left: &Node,
    _operator: &str,
    _right: &Node,
    _requires: &[Node],
    _span: &Span,
) -> Option<Lint> {
    None
}

// ============================================================
// Shared AST walker. Not exhaustive — covers the shapes the
// five lints actually need to descend through.
// ============================================================

fn recurse_children<F: FnMut(&Node)>(node: &Node, f: &mut F) {
    match node {
        Node::Program(stmts) => {
            for s in stmts {
                f(&s.node);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            f(body);
            for r in requires {
                f(r);
            }
            for e in ensures {
                f(e);
            }
        }
        // RES-239: descend into impl block methods so L0003/L0004/L0006 cover methods.
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                f(method);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                f(s);
            }
        }
        Node::LetStatement { value, .. } | Node::StaticLet { value, .. } => f(value),
        Node::ReturnStatement { value: Some(v), .. } => f(v),
        Node::Assignment { value, .. } => f(value),
        Node::ExpressionStatement { expr, .. } => f(expr),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            f(condition);
            f(consequence);
            if let Some(a) = alternative {
                f(a);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            f(condition);
            f(body);
        }
        Node::ForInStatement { iterable, body, .. } => {
            f(iterable);
            f(body);
        }
        Node::LiveBlock {
            body, invariants, ..
        } => {
            f(body);
            for inv in invariants {
                f(inv);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            f(left);
            f(right);
        }
        Node::PrefixExpression { right, .. } => f(right),
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            f(function);
            for a in arguments {
                f(a);
            }
        }
        Node::TryExpression { expr, .. } => f(expr),
        Node::OptionalChain { object, access, .. } => {
            f(object);
            if let crate::ChainAccess::Method(_, args) = access {
                for a in args {
                    f(a);
                }
            }
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            f(scrutinee);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    f(g);
                }
                f(arm_body);
            }
        }
        Node::IndexExpression { target, index, .. } => {
            f(target);
            f(index);
        }
        Node::IndexAssignment {
            target,
            index,
            value,
            ..
        } => {
            f(target);
            f(index);
            f(value);
        }
        Node::FieldAccess { target, .. } => f(target),
        Node::FieldAssignment { target, value, .. } => {
            f(target);
            f(value);
        }
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                f(i);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                f(v);
            }
        }
        Node::Assert { condition, .. } => f(condition),
        Node::Assume {
            condition, message, ..
        } => {
            f(condition);
            if let Some(msg) = message {
                f(msg);
            }
        }
        _ => {}
    }
}

// ============================================================
// Suppress-comment scanning
// ============================================================
//
// Finds every `// resilient: allow LXXXX` line in the source
// and returns the set of `(line, code)` pairs that should be
// suppressed. An allow on line K suppresses diagnostics on line
// K+1. Only `L` codes are recognized; `// resilient: allow foo`
// is treated as ordinary text.

fn collect_allow_comments(source: &str) -> std::collections::HashSet<(u32, String)> {
    let mut out = std::collections::HashSet::new();
    for (i, raw) in source.lines().enumerate() {
        let line_no = (i as u32) + 1;
        let Some(pos) = raw.find("// resilient: allow") else {
            continue;
        };
        let tail = &raw[pos + "// resilient: allow".len()..];
        // Collect every LXXXX token on the rest of the line.
        for word in tail.split(|c: char| c == ',' || c.is_whitespace()) {
            let w = word.trim();
            if w.starts_with('L') && w.len() == 5 && w.chars().skip(1).all(|c| c.is_ascii_digit()) {
                out.insert((line_no + 1, w.to_string()));
            }
        }
    }
    out
}

// ============================================================
// L0010: function has no requires/ensures contract
// ============================================================
//
// Functions that declare neither `requires` nor `ensures` carry
// no machine-verifiable safety contract.  In safety-critical
// embedded code that is almost always an oversight, so we flag
// it as a warning.  Users can suppress with:
//   `// resilient: allow L0010`
// or add trivial stubs (the LSP `codeAction` offers this as a
// quick-fix: "Add contract stubs").
//
// Deliberately excluded from the check:
//   - Functions that start with `_` (test helpers, entry stubs).
//   - Anonymous functions (name == "").
//   - Impl-block methods (those inherit the struct's invariants).

fn run_l0010_no_contract(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        if let Node::Function {
            name,
            requires,
            ensures,
            span,
            ..
        } = &spanned.node
        {
            // Skip anonymous fns and underscore-prefixed helpers.
            if name.is_empty() || name.starts_with('_') {
                continue;
            }
            if requires.is_empty() && ensures.is_empty() {
                out.push(Lint {
                    code: "L0010".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "function `{name}` has no `requires`/`ensures` contract; \
                         add contract stubs or suppress with `// resilient: allow L0010`"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
}

// ============================================================
// L0011: unused variable warning (RES-308)
// ============================================================
//
// Issue #100 / RES-308 specifies a dedicated lint for `let`
// bindings whose name is never subsequently read. The earlier
// L0001 lint also flags this case (it covers all "unused local
// binding" forms — `let`, `for`-loop vars, struct-destructure,
// match-arm bindings) but the ticket specifically asks for a
// distinct code with the rustc-style message
// `variable \`x\` is assigned but never used`.
//
// `KNOWN_CODES` already reserves `L0002` for "unreachable arm
// after `_`" with a substantial test suite; per the
// repo-wide test-protection rule we cannot retire that code
// without breaking unrelated tests, so the new lint is
// allocated the next free slot, `L0011`.
//
// Behaviour:
//   - Walks every `let` / `static let` / struct-destructure
//     binding inside fn bodies (incl. impl methods).
//   - A binding is "used" if its name appears in any
//     identifier-read position elsewhere in the same fn body.
//   - Names starting with `_` are exempt.
//   - Reports at the binding's source span — the same site the
//     ticket asks about (`file:line:col`).
//
// `for x in arr` loop variables are intentionally skipped here
// because L0001 already covers them and the ticket's wording
// ("`let x = expr;`") doesn't mention them. Match-arm pattern
// bindings are also out of scope (L0001 covers those).

fn run_l0011_unused_variable(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function { body, .. } => {
                l0011_check_body(body, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, .. } = method {
                        l0011_check_body(body, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0011_check_body(body: &Node, out: &mut Vec<Lint>) {
    // RES-1533: same borrow pattern as `l0001_check_body`.
    let mut lets: Vec<(&str, Span)> = Vec::new();
    l0011_collect_let_bindings(body, &mut lets);
    if lets.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
    collect_identifier_reads_in(body, &mut used);
    for (name, span) in &lets {
        if name.starts_with('_') {
            continue;
        }
        if !used.contains(*name) {
            out.push(Lint {
                code: "L0011".into(),
                severity: Severity::Warning,
                message: format!("variable `{}` is assigned but never used", name),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

/// L0011-specific binding collector. Mirrors `collect_lets_in` but
/// scoped to the let-style forms named by the ticket: plain `let`,
/// `static let`, and struct-destructure. `for`-loop induction
/// variables are deliberately skipped — L0001 already flags those.
fn l0011_collect_let_bindings<'a>(node: &'a Node, out: &mut Vec<(&'a str, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.as_str(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::LetDestructureStruct {
            fields,
            value,
            span,
            ..
        } => {
            for (_field_name, local_name) in fields {
                out.push((local_name.as_str(), *span));
            }
            l0011_collect_let_bindings(value, out);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0011_collect_let_bindings(s, out);
            }
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0011_collect_let_bindings(condition, out);
            l0011_collect_let_bindings(consequence, out);
            if let Some(a) = alternative {
                l0011_collect_let_bindings(a, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0011_collect_let_bindings(condition, out);
            l0011_collect_let_bindings(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0011_collect_let_bindings(iterable, out);
            l0011_collect_let_bindings(body, out);
        }
        Node::LiveBlock { body, .. } => l0011_collect_let_bindings(body, out),
        Node::Match {
            scrutinee, arms, ..
        } => {
            l0011_collect_let_bindings(scrutinee, out);
            for (_, guard, arm_body) in arms {
                if let Some(g) = guard {
                    l0011_collect_let_bindings(g, out);
                }
                l0011_collect_let_bindings(arm_body, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0012: spec annotation lacks `// source:` provenance comment (RES-397)
// ============================================================
//
// Reddit critique (https://www.reddit.com/r/VibeCodersNest/comments/1ssv8ih/)
// raised the strongest version of "filtering ≠ safety": if an LLM
// invents the invariant, a self-consistent wrong spec is provable
// and useless. The verification machinery is sound — it doesn't
// trust the LLM — but the *invariants themselves* have no
// provenance trail today. A wrong invariant from an LLM is
// indistinguishable from a right one once it's in the source.
//
// L0012 requires every spec-bearing site to be preceded by a
// `// source: <canonical-reference>` comment on the line above:
//
//   // source: RFC 9293 §3.5
//   fn handle_segment(seq: int) requires seq >= 0 { ... }
//
//   // source: STM32F4 Reference Manual RM0090 §10.4.5
//   assume(adc_value < 4096);
//
// Sites covered:
//   - Function declarations with non-empty `requires`, `ensures`,
//     `recovers_to`, or `fails`.
//   - `assume(...)` statements.
//
// Suppress with `// resilient: allow L0012`. The default severity
// is Warning; `--deny L0012` escalates to Error.

/// RES-397: collect line numbers that have a spec annotation on
/// them, given a `// source: ...` comment on the line above. The
/// returned set contains `K+1` for every `// source: ...` on line
/// `K`. This mirrors the line-offset convention used by
/// `collect_allow_comments`.
fn collect_source_comments(source: &str) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    for (i, raw) in source.lines().enumerate() {
        let line_no = (i as u32) + 1;
        let trimmed = raw.trim_start();
        if let Some(rest) = trimmed.strip_prefix("// source:")
            && !rest.trim().is_empty()
        {
            out.insert(line_no + 1);
        }
    }
    out
}

fn run_l0012_spec_provenance(program: &Node, source: &str, out: &mut Vec<Lint>) {
    let sources = collect_source_comments(source);
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        l0012_walk(&spanned.node, &sources, out);
    }
}

fn l0012_walk(node: &Node, sources: &std::collections::HashSet<u32>, out: &mut Vec<Lint>) {
    match node {
        Node::Function {
            name,
            requires,
            ensures,
            recovers_to,
            fails,
            body,
            span,
            ..
        } => {
            let has_spec = !requires.is_empty()
                || !ensures.is_empty()
                || recovers_to.is_some()
                || !fails.is_empty();
            if has_spec && !name.is_empty() && !name.starts_with('_') {
                let fn_line = span.start.line as u32;
                if !sources.contains(&fn_line) {
                    out.push(Lint {
                        code: "L0012".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "function `{name}` has spec annotations without provenance — \
                             add `// source: <canonical-reference>` on the line above, \
                             or suppress with `// resilient: allow L0012`"
                        ),
                        line: fn_line,
                        column: span.start.column as u32,
                    });
                }
            }
            l0012_walk(body, sources, out);
        }
        Node::Assume { span, .. } => {
            let line = span.start.line as u32;
            if !sources.contains(&line) {
                out.push(Lint {
                    code: "L0012".into(),
                    severity: Severity::Warning,
                    message: "`assume()` without provenance — \
                              add `// source: <canonical-reference>` on the line above, \
                              or suppress with `// resilient: allow L0012`"
                        .to_string(),
                    line,
                    column: span.start.column as u32,
                });
            }
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                l0012_walk(stmt, sources, out);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            l0012_walk(consequence, sources, out);
            if let Some(alt) = alternative {
                l0012_walk(alt, sources, out);
            }
        }
        Node::WhileStatement { body, .. } => l0012_walk(body, sources, out),
        Node::ForInStatement { body, .. } => l0012_walk(body, sources, out),
        Node::LiveBlock { body, .. } => l0012_walk(body, sources, out),
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                l0012_walk(method, sources, out);
            }
        }
        _ => {}
    }
}

fn run_l0013_unchecked_indexing(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        l0013_walk(&spanned.node, out);
    }
}

fn l0013_walk(node: &Node, out: &mut Vec<Lint>) {
    match node {
        Node::IndexExpression {
            target,
            index,
            span,
            ..
        } => {
            // RES-798: check if this index access was proven in-bounds by
            // the bounds_check pass. If not, emit L0013 warning.
            if !crate::bounds_check::is_proven_site(*span) {
                out.push(Lint {
                    code: "L0013".into(),
                    severity: Severity::Warning,
                    message: "unchecked array indexing — bounds not proven at compile time; \
                         use --deny-unproven-bounds to require proof, or suppress with \
                         `// resilient: allow L0013`"
                        .to_string(),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
            // Recurse into both target and index
            l0013_walk(target, out);
            l0013_walk(index, out);
        }
        Node::Block { stmts, .. } => {
            for stmt in stmts {
                l0013_walk(stmt, out);
            }
        }
        Node::Function {
            body,
            requires,
            ensures,
            ..
        } => {
            for req in requires {
                l0013_walk(req, out);
            }
            for ens in ensures {
                l0013_walk(ens, out);
            }
            l0013_walk(body, out);
        }
        Node::IfStatement {
            consequence,
            alternative,
            condition,
            ..
        } => {
            l0013_walk(condition, out);
            l0013_walk(consequence, out);
            if let Some(alt) = alternative {
                l0013_walk(alt, out);
            }
        }
        Node::WhileStatement {
            body, condition, ..
        } => {
            l0013_walk(condition, out);
            l0013_walk(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0013_walk(iterable, out);
            l0013_walk(body, out);
        }
        Node::LiveBlock { body, .. } => {
            l0013_walk(body, out);
        }
        Node::Match {
            scrutinee, arms, ..
        } => {
            l0013_walk(scrutinee, out);
            for (_, guard, body) in arms {
                if let Some(g) = guard {
                    l0013_walk(g, out);
                }
                l0013_walk(body, out);
            }
        }
        Node::InfixExpression { left, right, .. } => {
            l0013_walk(left, out);
            l0013_walk(right, out);
        }
        Node::PrefixExpression { right, .. } => {
            l0013_walk(right, out);
        }
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            l0013_walk(function, out);
            for arg in arguments {
                l0013_walk(arg, out);
            }
        }
        Node::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                l0013_walk(value, out);
            }
        }
        Node::ReturnStatement {
            value: Some(val), ..
        } => {
            l0013_walk(val, out);
        }
        Node::ReturnStatement { value: None, .. } => {}
        Node::ImplBlock { methods, .. } => {
            for method in methods {
                l0013_walk(method, out);
            }
        }
        Node::FieldAccess { target, .. } => {
            l0013_walk(target, out);
        }
        Node::ExpressionStatement { expr, .. } => {
            l0013_walk(expr, out);
        }
        Node::TryCatch { body, handlers, .. } => {
            for stmt in body {
                l0013_walk(stmt, out);
            }
            for (_, handler_body) in handlers {
                for stmt in handler_body {
                    l0013_walk(stmt, out);
                }
            }
        }
        Node::ArrayLiteral { items, .. } => {
            for item in items {
                l0013_walk(item, out);
            }
        }
        _ => {}
    }
}

// L0014: function defined but never called (dead function)
//
// Collects every top-level function name and every call-target
// identifier anywhere in the program.  Any function that was defined
// but whose name never appears as a callee is warned.
//
// Exceptions:
// * `_`-prefixed names (silenced by convention, same as L0001/L0011).
// * Names that appear as identifiers outside of call position (e.g.
//   passed as higher-order values) are treated as "used" — the lint
//   focuses on the unambiguous dead-function case.
fn run_l0014_unused_function(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };

    // Phase 1: collect (name, span) for every top-level fn definition.
    let mut defined: Vec<(&str, Span)> = Vec::new();
    for spanned in stmts {
        if let Node::Function { name, span, .. } = &spanned.node {
            defined.push((name.as_str(), *span));
        }
    }
    if defined.is_empty() {
        return;
    }

    // Phase 2: collect every identifier that appears as a call target
    // anywhere in the program (including top-level call statements).
    let mut called: std::collections::HashSet<&str> =
        std::collections::HashSet::with_capacity(defined.len());
    for spanned in stmts {
        l0014_collect_calls(&spanned.node, &mut called);
    }

    // Phase 3: warn for each defined fn whose name was never called.
    for (name, span) in defined {
        if name.starts_with('_') {
            continue;
        }
        if !called.contains(name) {
            out.push(Lint {
                code: "L0014".into(),
                severity: Severity::Warning,
                message: format!(
                    "function `{}` is defined but never called — prefix with `_` to silence",
                    name
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
}

/// Recursively collect all call-target identifiers in `node`.
fn l0014_collect_calls<'a>(node: &'a Node, out: &mut std::collections::HashSet<&'a str>) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                out.insert(name.as_str());
            }
            // Recurse into function expression itself (handles chained calls,
            // method dispatch, etc.) and into all arguments.
            l0014_collect_calls(function, out);
            for a in arguments {
                l0014_collect_calls(a, out);
            }
        }
        Node::Function { body, .. } => l0014_collect_calls(body, out),
        Node::Block { stmts, .. } => {
            for s in stmts {
                l0014_collect_calls(s, out);
            }
        }
        Node::LetStatement { value, .. }
        | Node::StaticLet { value, .. }
        | Node::Const { value, .. }
        | Node::Assignment { value, .. } => l0014_collect_calls(value, out),
        Node::ReturnStatement { value: Some(v), .. } => l0014_collect_calls(v, out),
        Node::ExpressionStatement { expr, .. } => l0014_collect_calls(expr, out),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0014_collect_calls(condition, out);
            l0014_collect_calls(consequence, out);
            if let Some(e) = alternative {
                l0014_collect_calls(e, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0014_collect_calls(condition, out);
            l0014_collect_calls(body, out);
        }
        Node::ForInStatement { iterable, body, .. } => {
            l0014_collect_calls(iterable, out);
            l0014_collect_calls(body, out);
        }
        Node::InfixExpression { left, right, .. } => {
            l0014_collect_calls(left, out);
            l0014_collect_calls(right, out);
        }
        Node::PrefixExpression { right, .. } => l0014_collect_calls(right, out),
        Node::ArrayLiteral { items, .. } => {
            for i in items {
                l0014_collect_calls(i, out);
            }
        }
        Node::FieldAccess { target, .. } => l0014_collect_calls(target, out),
        Node::FieldAssignment { target, value, .. } => {
            l0014_collect_calls(target, out);
            l0014_collect_calls(value, out);
        }
        Node::IndexExpression { target, index, .. } => {
            l0014_collect_calls(target, out);
            l0014_collect_calls(index, out);
        }
        Node::ImplBlock { methods, .. } => {
            for m in methods {
                l0014_collect_calls(m, out);
            }
        }
        _ => {}
    }
}

// ============================================================
// L0015: constant arithmetic expression overflows `int`
// ============================================================
//
// Fires when every operand of an arithmetic infix expression is a
// compile-time-known integer literal and the operation overflows
// signed 64-bit integer range.  Division/modulo by zero is already
// covered by L0009 and is not re-reported here.

fn run_l0015_const_overflow(program: &Node, out: &mut Vec<Lint>) {
    walk_l0015(program, out);
}

/// Try to evaluate `node` to a compile-time constant `i64`.
/// Returns `None` on any free identifier, function call, or
/// arithmetic overflow (so the caller can detect the overflow case
/// separately).
fn try_const_int(node: &Node) -> Option<i64> {
    match node {
        Node::IntegerLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "-" => try_const_int(right).and_then(i64::checked_neg),
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => {
            let l = try_const_int(left)?;
            let r = try_const_int(right)?;
            match operator.as_str() {
                "+" => l.checked_add(r),
                "-" => l.checked_sub(r),
                "*" => l.checked_mul(r),
                "/" => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_div(r)
                    }
                }
                "%" => {
                    if r == 0 {
                        None
                    } else {
                        l.checked_rem(r)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn walk_l0015(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        operator,
        left,
        right,
        span,
    } = node
    {
        let op = operator.as_str();
        if matches!(op, "+" | "-" | "*" | "/" | "%") {
            let l_val = try_const_int(left);
            let r_val = try_const_int(right);
            if let (Some(l), Some(r)) = (l_val, r_val) {
                let overflows = match op {
                    "+" => l.checked_add(r).is_none(),
                    "-" => l.checked_sub(r).is_none(),
                    "*" => l.checked_mul(r).is_none(),
                    // div/rem by zero → L0009, not L0015
                    "/" => r != 0 && l.checked_div(r).is_none(),
                    "%" => r != 0 && l.checked_rem(r).is_none(),
                    _ => false,
                };
                if overflows {
                    out.push(Lint {
                        code: "L0015".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "constant expression `{l} {op} {r}` overflows `int` — \
                             use smaller values or suppress with \
                             `// resilient: allow L0015`"
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                    return;
                }
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0015(child, out));
}

// ============================================================
// L0016: constant boolean condition in `if` statement
// ============================================================
//
// Fires when the condition of an `if` is a compile-time constant
// (`true`, `false`, or a fully-folded boolean expression).  This
// catches dead branches (`if false { ... }`) and tautological ones
// (`if true { ... }`) that should be simplified or removed.

fn run_l0016_constant_condition(program: &Node, out: &mut Vec<Lint>) {
    walk_l0016(program, out);
}

fn try_const_bool(node: &Node) -> Option<bool> {
    match node {
        Node::BooleanLiteral { value, .. } => Some(*value),
        Node::PrefixExpression {
            operator, right, ..
        } if operator == "!" => try_const_bool(right).map(|v| !v),
        Node::InfixExpression {
            operator,
            left,
            right,
            ..
        } => match operator.as_str() {
            "==" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l == r)
            }
            "!=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l != r)
            }
            "<" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l < r)
            }
            ">" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l > r)
            }
            "<=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l <= r)
            }
            ">=" => {
                let (l, r) = (try_const_int(left)?, try_const_int(right)?);
                Some(l >= r)
            }
            "&&" => match (try_const_bool(left), try_const_bool(right)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            "||" => match (try_const_bool(left), try_const_bool(right)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn walk_l0016(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        condition, span, ..
    } = node
        && let Some(val) = try_const_bool(condition)
    {
        let branch = if val { "always taken" } else { "never taken" };
        out.push(Lint {
            code: "L0016".into(),
            severity: Severity::Warning,
            message: format!(
                "condition is always `{val}` — this branch is {branch}; \
                 simplify or suppress with `// resilient: allow L0016`"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0016(child, out));
}

// ============================================================
// L0017: variable shadowing
// ============================================================
//
// Fires when a `let` binding in an inner scope uses the same name
// as a binding in any enclosing scope (parameters or outer let).
// Names starting with `_` are exempt — the leading underscore is
// the conventional "I know this shadows" signal.

fn run_l0017_variable_shadowing(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                parameters, body, ..
            } => {
                let mut scopes: Vec<std::collections::HashSet<String>> =
                    vec![parameters.iter().map(|(_, name)| name.clone()).collect()];
                l0017_walk(body, &mut scopes, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        parameters, body, ..
                    } = method
                    {
                        let mut scopes: Vec<std::collections::HashSet<String>> =
                            vec![parameters.iter().map(|(_, name)| name.clone()).collect()];
                        l0017_walk(body, &mut scopes, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0017_walk(
    node: &Node,
    scopes: &mut Vec<std::collections::HashSet<String>>,
    out: &mut Vec<Lint>,
) {
    match node {
        Node::Block { stmts, .. } => {
            scopes.push(std::collections::HashSet::new());
            for stmt in stmts {
                l0017_walk(stmt, scopes, out);
            }
            scopes.pop();
        }
        Node::LetStatement {
            name, value, span, ..
        } => {
            if !name.starts_with('_') {
                let outer_len = scopes.len().saturating_sub(1);
                let shadows = scopes[..outer_len]
                    .iter()
                    .any(|s| s.contains(name.as_str()));
                if shadows {
                    out.push(Lint {
                        code: "L0017".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "variable `{}` shadows a previous declaration — \
                             rename to avoid confusion, or prefix with `_` to silence",
                            name
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
            }
            if let Some(top) = scopes.last_mut() {
                top.insert(name.clone());
            }
            l0017_walk(value, scopes, out);
        }
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            l0017_walk(condition, scopes, out);
            l0017_walk(consequence, scopes, out);
            if let Some(alt) = alternative {
                l0017_walk(alt, scopes, out);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            l0017_walk(condition, scopes, out);
            l0017_walk(body, scopes, out);
        }
        Node::ForInStatement { body, iterable, .. } => {
            l0017_walk(iterable, scopes, out);
            l0017_walk(body, scopes, out);
        }
        Node::ReturnStatement { value, .. } => {
            if let Some(v) = value {
                l0017_walk(v, scopes, out);
            }
        }
        Node::ExpressionStatement { expr, .. } => {
            l0017_walk(expr, scopes, out);
        }
        Node::Assignment { value, .. } => {
            l0017_walk(value, scopes, out);
        }
        // Nested function definitions have independent scopes; don't
        // carry the outer scope stack into them.
        Node::Function { .. } => {}
        _ => {
            recurse_children(node, &mut |child| l0017_walk(child, scopes, out));
        }
    }
}

// ============================================================
// L0018: missing return on all paths
// ============================================================
//
// Fires for functions with an explicit `-> TYPE` annotation (where
// TYPE is not `void`) whose body does not return on every code path.
// Heuristic: a block "returns on all paths" when its last statement
// is a `return`, or is an `if/else` where both branches return.
// A function with no else clause, or that falls off the end of its
// body, gets a warning.

fn run_l0018_missing_return(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                name,
                return_type,
                body,
                span,
                ..
            } => {
                if let Some(rt) = return_type
                    && !l0018_is_void(rt)
                    && !l0018_all_paths_return(body)
                {
                    out.push(Lint {
                        code: "L0018".into(),
                        severity: Severity::Warning,
                        message: format!(
                            "function `{}` has return type `{}` but may not return \
                             on all paths — add a `return` or suppress with \
                             `// resilient: allow L0018`",
                            name, rt
                        ),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        name,
                        return_type,
                        body,
                        span,
                        ..
                    } = method
                        && let Some(rt) = return_type
                        && !l0018_is_void(rt)
                        && !l0018_all_paths_return(body)
                    {
                        out.push(Lint {
                            code: "L0018".into(),
                            severity: Severity::Warning,
                            message: format!(
                                "function `{}` has return type `{}` but may not \
                                 return on all paths",
                                name, rt
                            ),
                            line: span.start.line as u32,
                            column: span.start.column as u32,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0018_is_void(rt: &str) -> bool {
    matches!(rt.trim(), "void" | "()" | "")
}

/// Returns `true` when `node` is guaranteed to produce a value (explicit
/// `return` or implicit expression result) on every execution path through it.
/// Conservative — never false-positive.
fn l0018_all_paths_return(node: &Node) -> bool {
    match node {
        Node::ReturnStatement { .. } => true,
        // An expression statement at the tail of a block is Resilient's
        // implicit-return form (same as Rust's expression-oriented blocks).
        // `fn f() -> int { a + b }` is valid — `a + b` IS the return value.
        Node::ExpressionStatement { .. } => true,
        Node::Block { stmts, .. } => {
            // A block returns on all paths if any statement in it does (once
            // a return is reached, subsequent stmts are unreachable).
            stmts.iter().any(l0018_all_paths_return)
        }
        // Returns on all paths only when both branches cover all paths.
        // No `else` means the `if`-false path falls through.
        Node::IfStatement {
            consequence,
            alternative: Some(alt),
            ..
        } => l0018_all_paths_return(consequence) && l0018_all_paths_return(alt),
        Node::IfStatement {
            alternative: None, ..
        } => false,
        // A while/for loop body might not execute at all, so it doesn't
        // guarantee a return.
        Node::WhileStatement { .. } | Node::ForInStatement { .. } => false,
        _ => false,
    }
}

// ============================================================
// L0019: format() argument count mismatch
// ============================================================
//
// `format(template, args_array)` takes exactly two arguments.
// Fires when:
//   (a) The call has != 2 arguments, OR
//   (b) The template is a static string (no runtime interpolation)
//       and args is an array literal, and the placeholder count
//       doesn't match the array length.
//
// Notes on AST shape:
//   - A Resilient template like `"\{} \{}"` stores `\{` as an
//     "unknown escape" in the lexer; string_interp's parse_parts
//     converts `\{` → `{`, so the InterpolatedString's Literal
//     parts already contain `{}` as the placeholder text.
//   - A template with no braces (e.g. `"hello"`) is a plain
//     StringLiteral; parse_template sees no placeholders.
//   - Templates with runtime interpolation (`"{expr}"`) have
//     Expr parts — arity is not statically checkable.

fn run_l0019_format_arity(program: &Node, out: &mut Vec<Lint>) {
    walk_l0019(program, out);
}

/// Extract the concatenated literal text from a template node, if it
/// has no runtime-interpolation `Expr` parts.
fn l0019_literal_template(node: &Node) -> Option<String> {
    match node {
        Node::StringLiteral { value, .. } => Some(value.clone()),
        Node::InterpolatedString { parts, .. } => {
            if parts
                .iter()
                .all(|p| matches!(p, crate::string_interp::StringPart::Literal(_)))
            {
                Some(
                    parts
                        .iter()
                        .map(|p| match p {
                            crate::string_interp::StringPart::Literal(s) => s.as_str(),
                            _ => "",
                        })
                        .collect(),
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

fn walk_l0019(node: &Node, out: &mut Vec<Lint>) {
    if let Node::CallExpression {
        function,
        arguments,
        span,
    } = node
        && let Node::Identifier { name, .. } = function.as_ref()
        && name == "format"
    {
        if arguments.len() != 2 {
            out.push(Lint {
                code: "L0019".into(),
                severity: Severity::Warning,
                message: format!(
                    "format() requires exactly 2 arguments (template, args_array) but {} {} supplied \
                     — suppress with `// resilient: allow L0019`",
                    arguments.len(),
                    if arguments.len() == 1 { "was" } else { "were" },
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        } else if let Some(tmpl) = l0019_literal_template(&arguments[0])
            && let Node::ArrayLiteral { items, .. } = &arguments[1]
            && let Ok(segments) = crate::format_builtin::parse_template(&tmpl)
        {
            let placeholders = segments
                .iter()
                .filter(|s| matches!(s, crate::format_builtin::FormatSegment::Placeholder(_)))
                .count();
            let array_len = items.len();
            if placeholders != array_len {
                out.push(Lint {
                    code: "L0019".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "format() template has {} placeholder{} but args array has {} element{} \
                         — counts must match; suppress with `// resilient: allow L0019`",
                        placeholders,
                        if placeholders == 1 { "" } else { "s" },
                        array_len,
                        if array_len == 1 { "" } else { "s" },
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0019(child, out));
}

// ============================================================
// L0020: unused function parameter
// ============================================================
//
// For each `fn`, collect parameter names and check whether each
// appears in the body (or in `requires`/`ensures` clauses).
// `_`-prefixed params are intentionally silenced by convention.
// Parameters that appear only in `requires`/`ensures` (pre/post-
// conditions) are considered used — they constrain the contract
// even if the body doesn't directly reference them.

fn run_l0020_unused_parameter(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    for spanned in stmts {
        match &spanned.node {
            Node::Function {
                parameters,
                body,
                requires,
                ensures,
                span,
                ..
            } => {
                l0020_check_params(parameters, body, requires, ensures, span, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function {
                        parameters,
                        body,
                        requires,
                        ensures,
                        span,
                        ..
                    } = method
                    {
                        l0020_check_params(parameters, body, requires, ensures, span, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn l0020_check_params(
    parameters: &[(String, String)],
    body: &Node,
    requires: &[Node],
    ensures: &[Node],
    fn_span: &Span,
    out: &mut Vec<Lint>,
) {
    if parameters.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<&str> = std::collections::HashSet::new();
    collect_identifier_reads_in(body, &mut used);
    for req in requires {
        collect_identifier_reads_in(req, &mut used);
    }
    for ens in ensures {
        collect_identifier_reads_in(ens, &mut used);
    }
    for (_ty, pname) in parameters {
        if pname.starts_with('_') {
            continue;
        }
        if !used.contains(pname.as_str()) {
            out.push(Lint {
                code: "L0020".into(),
                severity: Severity::Warning,
                message: format!("unused parameter `{}` — prefix with `_` to silence", pname),
                line: fn_span.start.line as u32,
                column: fn_span.start.column as u32,
            });
        }
    }
}

// ============================================================
// L0021: redundant boolean sub-expression (x && x, x || x)
// ============================================================
//
// Detects infix `&&` or `||` where both operands are structurally
// identical (same identifier or same literal). The always-true
// tautology `x || x` and always-redundant `x && x` are bugs or
// dead code. This extends L0003 (which catches `x == x`) to
// logical operators.

fn run_l0021_redundant_bool_subexpr(program: &Node, out: &mut Vec<Lint>) {
    walk_l0021(program, out);
}

fn walk_l0021(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "&&" || operator == "||")
        && nodes_structurally_equal(left, right)
    {
        out.push(Lint {
            code: "L0021".into(),
            severity: Severity::Warning,
            message: format!(
                "redundant sub-expression: both sides of `{}` are identical",
                operator
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0021(child, out));
}

/// Shallow structural equality for L0021.
/// Two nodes are equal if they are:
/// - The same `Identifier` (same name)
/// - The same `IntegerLiteral` / `FloatLiteral` / `BooleanLiteral`
/// - The same `StringLiteral`
/// - The same `PrefixExpression` with equal operands
/// - The same `InfixExpression` with equal operands
fn nodes_structurally_equal(a: &Node, b: &Node) -> bool {
    match (a, b) {
        (Node::Identifier { name: na, .. }, Node::Identifier { name: nb, .. }) => na == nb,
        (Node::IntegerLiteral { value: va, .. }, Node::IntegerLiteral { value: vb, .. }) => {
            va == vb
        }
        (Node::FloatLiteral { value: va, .. }, Node::FloatLiteral { value: vb, .. }) => {
            va.to_bits() == vb.to_bits()
        }
        (Node::BooleanLiteral { value: va, .. }, Node::BooleanLiteral { value: vb, .. }) => {
            va == vb
        }
        (Node::StringLiteral { value: va, .. }, Node::StringLiteral { value: vb, .. }) => va == vb,
        (
            Node::PrefixExpression {
                operator: oa,
                right: ra,
                ..
            },
            Node::PrefixExpression {
                operator: ob,
                right: rb,
                ..
            },
        ) => oa == ob && nodes_structurally_equal(ra, rb),
        (
            Node::InfixExpression {
                left: la,
                operator: oa,
                right: ra,
                ..
            },
            Node::InfixExpression {
                left: lb,
                operator: ob,
                right: rb,
                ..
            },
        ) => oa == ob && nodes_structurally_equal(la, lb) && nodes_structurally_equal(ra, rb),
        _ => false,
    }
}

// ============================================================
// L0022: needless else after unconditional return
// ============================================================
//
// Detects `if cond { return x; } else { ... }` where the
// consequence block always returns. The `else` keyword is
// redundant because control flow after the if-block already
// implies the condition was false. Removing the `else` and
// de-indenting the body is cleaner and avoids confusion.

fn run_l0022_needless_else(program: &Node, out: &mut Vec<Lint>) {
    walk_l0022(program, out);
}

fn walk_l0022(node: &Node, out: &mut Vec<Lint>) {
    if let Node::IfStatement {
        consequence,
        alternative: Some(alt),
        span,
        ..
    } = node
    {
        if l0018_all_paths_return(consequence) {
            out.push(Lint {
                code: "L0022".into(),
                severity: Severity::Warning,
                message: "else block is redundant after a block that always returns; \
                          remove the `else` and de-indent the body"
                    .into(),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
        // Still recurse into the alternative since nested ifs can also trigger.
        walk_l0022(alt, out);
    }
    recurse_children(node, &mut |child| walk_l0022(child, out));
}

// ============================================================
// L0023: tautological comparison with boolean literal
// ============================================================
//
// Detects `expr == true`, `expr == false`, `true == expr`,
// `false == expr`. These comparisons are always redundant:
// - `x == true`  → use `x` directly
// - `x == false` → use `!x`
// The reversed forms (literal on the left) are also caught.

fn run_l0023_bool_literal_comparison(program: &Node, out: &mut Vec<Lint>) {
    walk_l0023(program, out);
}

fn walk_l0023(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && operator == "=="
    {
        let (bool_val, other_side) = if let Node::BooleanLiteral { value, .. } = left.as_ref() {
            (Some(*value), right.as_ref())
        } else if let Node::BooleanLiteral { value, .. } = right.as_ref() {
            (Some(*value), left.as_ref())
        } else {
            (None, left.as_ref())
        };

        if let Some(literal) = bool_val {
            // Skip `true == true` / `false == false` (caught by L0003 or trivially obvious).
            if matches!(other_side, Node::BooleanLiteral { .. }) {
                // Let L0003 handle identical-operand case.
            } else {
                let suggestion = if literal {
                    "use the expression directly instead of `== true`"
                } else {
                    "use `!expr` instead of `== false`"
                };
                out.push(Lint {
                    code: "L0023".into(),
                    severity: Severity::Warning,
                    message: format!("tautological comparison with `{}`; {}", literal, suggestion),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0023(child, out));
}

// ============================================================
// L0025: unreachable code after infinite while-true loop
// ============================================================
//
// A `while true { ... }` loop that never `break`s or `return`s
// from the enclosing function makes all subsequent statements
// in the same block unreachable. Extends L0007 (unreachable
// after explicit `return`) to cover the loop variant.

fn run_l0025_unreachable_after_infinite_loop(program: &Node, out: &mut Vec<Lint>) {
    walk_l0025(program, out);
}

fn walk_l0025(node: &Node, out: &mut Vec<Lint>) {
    if let Node::Block { stmts, .. } = node {
        let mut found_infinite = false;
        for stmt in stmts {
            if found_infinite {
                if let Some(span) = node_span(stmt) {
                    out.push(Lint {
                        code: "L0025".into(),
                        severity: Severity::Warning,
                        message: "unreachable code after infinite `while true` loop".into(),
                        line: span.start.line as u32,
                        column: span.start.column as u32,
                    });
                }
                // Only report the first unreachable statement (same as L0007).
                break;
            }
            if is_infinite_while(stmt) {
                found_infinite = true;
            }
            walk_l0025(stmt, out);
        }
        if !found_infinite {
            for stmt in stmts {
                walk_l0025(stmt, out);
            }
        }
    } else {
        recurse_children(node, &mut |child| walk_l0025(child, out));
    }
}

/// True when `node` is a `while true { ... }` loop whose body
/// never breaks out via `break` (returns are fine — they exit the
/// whole function, making *everything* after the loop unreachable).
fn is_infinite_while(node: &Node) -> bool {
    let Node::WhileStatement {
        condition, body, ..
    } = node
    else {
        return false;
    };
    if !matches!(condition.as_ref(), Node::BooleanLiteral { value: true, .. }) {
        return false;
    }
    !l0025_body_has_break(body)
}

fn l0025_body_has_break(node: &Node) -> bool {
    match node {
        Node::Break { .. } => true,
        // Don't cross function boundaries.
        Node::Function { .. } => false,
        _ => {
            let mut found = false;
            recurse_children(node, &mut |child| {
                if !found {
                    found = l0025_body_has_break(child);
                }
            });
            found
        }
    }
}

/// Extract the source span from common statement nodes (best-effort).
fn node_span(node: &Node) -> Option<&Span> {
    match node {
        Node::LetStatement { span, .. }
        | Node::ReturnStatement { span, .. }
        | Node::IfStatement { span, .. }
        | Node::WhileStatement { span, .. }
        | Node::ForInStatement { span, .. }
        | Node::ExpressionStatement { span, .. }
        | Node::Assignment { span, .. } => Some(span),
        _ => None,
    }
}

// ============================================================
// L0024: struct literal missing required fields
// ============================================================
//
// Collects all `StructDecl` definitions visible at program scope,
// then walks every `StructLiteral` and warns when a declared field
// is absent from the literal. This is a lint-level warning (the
// typechecker will also error); the lint fires first and lists the
// missing names so the user can see at a glance what to add.

fn run_l0024_struct_missing_fields(program: &Node, out: &mut Vec<Lint>) {
    let Node::Program(stmts) = program else {
        return;
    };
    // Build struct-name → declared field names from top-level decls.
    let mut decls: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for spanned in stmts {
        if let Node::StructDecl { name, fields, .. } = &spanned.node {
            // `fields` is Vec<(type_name, field_name)>
            decls.insert(
                name.as_str(),
                fields.iter().map(|(_, fname)| fname.as_str()).collect(),
            );
        }
        // Descend into impl blocks — they don't contain StructDecls but
        // let the struct-collection pass stay consistent.
    }
    if decls.is_empty() {
        return;
    }
    walk_l0024(program, &decls, out);
}

fn walk_l0024<'a>(
    node: &'a Node,
    decls: &std::collections::HashMap<&str, Vec<&'a str>>,
    out: &mut Vec<Lint>,
) {
    if let Node::StructLiteral { name, fields, span } = node
        && let Some(declared) = decls.get(name.as_str())
    {
        let provided: std::collections::HashSet<&str> =
            fields.iter().map(|(fname, _)| fname.as_str()).collect();
        let missing: Vec<&str> = declared
            .iter()
            .filter(|f| !provided.contains(**f))
            .copied()
            .collect();
        if !missing.is_empty() {
            let list = missing
                .iter()
                .map(|f| format!("`{f}`"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push(Lint {
                code: "L0024".into(),
                severity: Severity::Warning,
                message: format!("struct literal `{name}` is missing required field(s): {list}"),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0024(child, decls, out));
}

// ============================================================
// L0026: duplicate literal key in map literal
// ============================================================
//
// When a map literal like `{ "a": 1, "b": 2, "a": 3 }` contains
// two entries with the same literal key, the first is silently
// overwritten at runtime. This is almost always a copy-paste
// mistake and never intentional.
//
// Only literal keys (string, integer, bool) are checked — dynamic
// expression keys can't be compared at lint time.

fn run_l0026_duplicate_map_key(program: &Node, out: &mut Vec<Lint>) {
    walk_l0026(program, out);
}

fn walk_l0026(node: &Node, out: &mut Vec<Lint>) {
    if let Node::MapLiteral { entries, span } = node {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (key, _) in entries {
            let repr = match key {
                Node::StringLiteral { value, .. } => Some(format!("\"{value}\"")),
                Node::IntegerLiteral { value, .. } => Some(value.to_string()),
                Node::BooleanLiteral { value, .. } => Some(value.to_string()),
                _ => None,
            };
            if let Some(k) = repr
                && !seen.insert(k.clone())
            {
                out.push(Lint {
                    code: "L0026".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "duplicate map key {k} — the earlier binding is silently overwritten"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0026(child, out));
}

// ============================================================
// L0027: empty catch block silently swallows errors
// ============================================================
//
// An empty `catch` arm (`catch (E) { }`) silently discards the
// error. Code that intentionally swallows should add a comment
// or a `let _e = ...` binding; this lint surfaces the pattern
// so it's visible during review.

fn run_l0027_empty_catch_block(program: &Node, out: &mut Vec<Lint>) {
    walk_l0027(program, out);
}

fn walk_l0027(node: &Node, out: &mut Vec<Lint>) {
    if let Node::TryCatch { handlers, span, .. } = node {
        for (error_type, body) in handlers {
            if body.is_empty() {
                out.push(Lint {
                    code: "L0027".into(),
                    severity: Severity::Warning,
                    message: format!(
                        "empty catch block for `{error_type}` silently discards the error — add a handler or re-raise"
                    ),
                    line: span.start.line as u32,
                    column: span.start.column as u32,
                });
            }
        }
    }
    recurse_children(node, &mut |child| walk_l0027(child, out));
}

// ============================================================
// L0028: negation of boolean literal (`!true` / `!false`)
// ============================================================
//
// `!true` always evaluates to `false` and `!false` always evaluates
// to `true`. Using the negated literal instead of the result literal
// is confusing and almost always indicates a logic error.

fn run_l0028_negation_of_literal(program: &Node, out: &mut Vec<Lint>) {
    walk_l0028(program, out);
}

fn walk_l0028(node: &Node, out: &mut Vec<Lint>) {
    if let Node::PrefixExpression {
        operator,
        right,
        span,
    } = node
        && operator == "!"
        && let Node::BooleanLiteral { value, .. } = right.as_ref()
    {
        let result = if *value { "false" } else { "true" };
        let literal = if *value { "true" } else { "false" };
        out.push(Lint {
            code: "L0028".into(),
            severity: Severity::Warning,
            message: format!("`!{literal}` is always `{result}` — use `{result}` directly"),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0028(child, out));
}

// ============================================================
// L0029: comparison result discarded as statement
// ============================================================
//
// An expression statement like `a == b;` computes a boolean but
// immediately discards the result. This is almost always a typo
// for an assignment (`a = b;`) or a missed assertion
// (`assert(a == b);`). For safety-critical code this pattern is
// particularly dangerous because a postcondition check silently
// becomes a no-op.

fn run_l0029_comparison_result_discarded(program: &Node, out: &mut Vec<Lint>) {
    walk_l0029(program, out);
}

fn walk_l0029(node: &Node, out: &mut Vec<Lint>) {
    if let Node::ExpressionStatement { expr, span } = node
        && let Node::InfixExpression { operator, .. } = expr.as_ref()
        && matches!(operator.as_str(), "==" | "!=" | "<" | "<=" | ">" | ">=")
    {
        out.push(Lint {
            code: "L0029".into(),
            severity: Severity::Warning,
            message: format!(
                "comparison `{operator}` result is discarded — did you mean `assert(…)` or `=`?"
            ),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0029(child, out));
}

// ============================================================
// L0030: float equality comparison (`==` / `!=`)
// ============================================================
//
// Comparing floats with `==` or `!=` is almost always a bug in
// safety-critical embedded code: floating-point arithmetic
// accumulates rounding error, so two computations that are
// mathematically equal will often produce different bit patterns.
// Use an epsilon comparison: `abs(a - b) < epsilon`.
//
// We fire only when at least one operand is a float literal; this
// covers the most common patterns (`x == 0.0`, `result != 1.5`)
// without requiring full type inference on both operands.

fn run_l0030_float_equality(program: &Node, out: &mut Vec<Lint>) {
    walk_l0030(program, out);
}

fn walk_l0030(node: &Node, out: &mut Vec<Lint>) {
    if let Node::InfixExpression {
        left,
        operator,
        right,
        span,
    } = node
        && (operator == "==" || operator == "!=")
    {
        let left_is_float = matches!(left.as_ref(), Node::FloatLiteral { .. });
        let right_is_float = matches!(right.as_ref(), Node::FloatLiteral { .. });
        if left_is_float || right_is_float {
            out.push(Lint {
                code: "L0030".into(),
                severity: Severity::Warning,
                message: format!(
                    "float equality comparison `{operator}` is almost always a bug — \
                     use an epsilon comparison: `abs(a - b) < epsilon`"
                ),
                line: span.start.line as u32,
                column: span.start.column as u32,
            });
        }
    }
    recurse_children(node, &mut |child| walk_l0030(child, out));
}

// ============================================================
// L0031: double negation `!!x`
// ============================================================
//
// `!!x` is semantically identical to `x` for any boolean `x`.
// The double negation is redundant and obscures intent; replace
// with the un-negated expression.

fn run_l0031_double_negation(program: &Node, out: &mut Vec<Lint>) {
    walk_l0031(program, out);
}

fn walk_l0031(node: &Node, out: &mut Vec<Lint>) {
    if let Node::PrefixExpression {
        operator,
        right,
        span,
    } = node
        && operator == "!"
        && let Node::PrefixExpression {
            operator: inner_op, ..
        } = right.as_ref()
        && inner_op == "!"
    {
        out.push(Lint {
            code: "L0031".into(),
            severity: Severity::Warning,
            message: "double negation `!!x` is redundant — use `x` directly".to_string(),
            line: span.start.line as u32,
            column: span.start.column as u32,
        });
    }
    recurse_children(node, &mut |child| walk_l0031(child, out));
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn lint(src: &str) -> Vec<Lint> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {errs:?}");
        check(&program, src)
    }

    fn codes(src: &str) -> Vec<String> {
        lint(src).into_iter().map(|l| l.code).collect()
    }

    // ---------- L0001: unused local binding ----------

    #[test]
    fn l0001_fires_on_unused_local() {
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        assert!(codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_when_local_is_used() {
        let src = "fn f(int a) {\n    let used = a + 1;\n    return used;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_for_underscore_prefix() {
        let src = "fn f(int a) {\n    let _ignored = 42;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_suppressed_by_allow_comment() {
        let src = "fn f(int a) {\n    // resilient: allow L0001\n    let unused = 42;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_fires_on_unused_for_in_loop_variable() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for item in arr {\n        return 1;\n    }\n}\n";
        assert!(codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_when_for_in_loop_variable_is_used() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for item in arr {\n        return item;\n    }\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    #[test]
    fn l0001_silent_for_underscore_prefixed_for_in_variable() {
        let src = "fn f() {\n    let arr = [1, 2, 3];\n    for _item in arr {\n        return 1;\n    }\n}\n";
        assert!(!codes(src).contains(&"L0001".to_string()));
    }

    // ---------- L0002: unreachable arm after _ ----------

    #[test]
    fn l0002_fires_on_arm_after_wildcard() {
        let src =
            "fn f(int n) {\n    return match n {\n        _ => 0,\n        1 => 1,\n    };\n}\n";
        assert!(codes(src).contains(&"L0002".to_string()));
    }

    #[test]
    fn l0002_silent_when_wildcard_is_last() {
        let src =
            "fn f(int n) {\n    return match n {\n        1 => 1,\n        _ => 0,\n    };\n}\n";
        assert!(!codes(src).contains(&"L0002".to_string()));
    }

    #[test]
    fn l0002_suppressed_by_allow_comment() {
        // The lint reports at the unreachable arm's body span,
        // so the allow comment goes on the line just above THAT
        // arm, not above the `match` keyword.
        let src = "fn f(int n) {\n    return match n {\n        _ => 0,\n        // resilient: allow L0002\n        1 => 1,\n    };\n}\n";
        assert!(!codes(src).contains(&"L0002".to_string()));
    }

    // ---------- L0002 / RES-232: Pattern::Bind as catch-all ----------

    #[test]
    fn l0002_fires_on_bind_with_wildcard_inner() {
        // `n @ _` is a catch-all; the arm after it is unreachable.
        let src = "fn f(int n) {\n    return match n {\n        n @ _ => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "expected L0002 for bind-with-wildcard-inner"
        );
    }

    #[test]
    fn l0002_fires_on_bind_with_identifier_inner() {
        // `n @ m` — inner is an identifier, also a catch-all.
        let src = "fn f(int n) {\n    return match n {\n        n @ m => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "expected L0002 for bind-with-identifier-inner"
        );
    }

    #[test]
    fn l0002_silent_on_bind_with_literal_inner() {
        // `n @ 5` is NOT a catch-all — it only matches the value 5.
        let src = "fn f(int n) {\n    return match n {\n        n @ 5 => 1,\n        0 => 2,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0002".to_string()),
            "L0002 must not fire for bind-with-literal-inner"
        );
    }

    // ---------- L0003: x == x ----------

    #[test]
    fn l0003_fires_on_self_eq() {
        let src = "fn f(int x) {\n    if x == x { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_fires_on_self_ne() {
        let src = "fn f(int x) {\n    if x != x { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_silent_on_distinct_operands() {
        let src = "fn f(int x, int y) {\n    if x == y { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0003".to_string()));
    }

    #[test]
    fn l0003_suppressed_by_allow_comment() {
        let src = "fn f(int x) {\n    // resilient: allow L0003\n    if x == x { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0003".to_string()));
    }

    // ---------- L0004: mixed && / || ----------

    #[test]
    fn l0004_fires_on_and_or_mix() {
        let src =
            "fn f(bool a, bool b, bool c) {\n    if a && b || c { return 1; }\n    return 0;\n}\n";
        assert!(codes(src).contains(&"L0004".to_string()));
    }

    #[test]
    fn l0004_silent_on_same_op() {
        let src =
            "fn f(bool a, bool b, bool c) {\n    if a && b && c { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0004".to_string()));
    }

    #[test]
    fn l0004_suppressed_by_allow_comment() {
        let src = "fn f(bool a, bool b, bool c) {\n    // resilient: allow L0004\n    if a && b || c { return 1; }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0004".to_string()));
    }

    // ---------- L0005: redundant trailing return ----------

    #[test]
    fn l0005_fires_on_trailing_bare_return() {
        let src = "fn f() {\n    let x = 1;\n    return;\n}\n";
        assert!(codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_silent_when_return_has_value() {
        let src = "fn f() {\n    return 1;\n}\n";
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_silent_when_no_return_stmt() {
        let src = "fn f() {\n    let x = 1;\n}\n";
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    #[test]
    fn l0005_suppressed_by_allow_comment() {
        let src = "fn f() {\n    let x = 1;\n    // resilient: allow L0005\n    return;\n}\n";
        // The allow is on the line directly above the bare `return;`.
        assert!(!codes(src).contains(&"L0005".to_string()));
    }

    // ---------- allow-comment parsing ----------

    #[test]
    fn allow_comment_accepts_multiple_codes_per_line() {
        let src = "fn f(int a) {\n    // resilient: allow L0001, L0005\n    let unused = 42;\n    return;\n}\n";
        // Both L0001 and L0005 should be silenced.
        let c = codes(src);
        // L0001 would fire at the `let` line (line 3).
        assert!(!c.contains(&"L0001".to_string()));
    }

    #[test]
    fn allow_comment_ignores_non_l_codes() {
        // "E0008" or "W0001" shouldn't be treated as an L code.
        let allows = collect_allow_comments("// resilient: allow E0008\n");
        assert!(allows.is_empty());
    }

    // ---------- format_lint ----------

    #[test]
    fn format_lint_uses_path_line_col_format() {
        let l = Lint {
            code: "L0001".into(),
            severity: Severity::Warning,
            message: "unused".into(),
            line: 5,
            column: 9,
        };
        let s = format_lint(&l, "src/thing.rs");
        assert_eq!(s, "src/thing.rs:5:9: warning[L0001]: unused");
    }

    #[test]
    fn known_codes_contains_all_five() {
        for code in ["L0001", "L0002", "L0003", "L0004", "L0005"] {
            assert!(KNOWN_CODES.contains(&code), "missing code: {code}");
        }
    }

    // ---------- composite ----------

    #[test]
    fn lints_sorted_by_line_column() {
        let src =
            "fn f(int x) {\n    if x == x { return 1; }\n    let unused = 42;\n    return 0;\n}\n";
        let out = lint(src);
        for pair in out.windows(2) {
            assert!(
                (pair[0].line, pair[0].column) <= (pair[1].line, pair[1].column),
                "lint order: {:?}",
                out,
            );
        }
    }

    #[test]
    fn empty_program_produces_no_lints() {
        assert!(lint("").is_empty());
    }

    // ---------- L0006: assume(false) vacuous discharge ----------

    #[test]
    fn l0006_fires_on_assume_false() {
        let src = "fn f() {\n    assume(false);\n}\n";
        assert!(codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_silent_on_assume_true() {
        let src = "fn f() {\n    assume(true);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_silent_on_assume_expr() {
        let src = "fn f(int x) {\n    assume(x > 0);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn l0006_suppressed_by_allow_comment() {
        let src = "fn f() {\n    // resilient: allow L0006\n    assume(false);\n}\n";
        assert!(!codes(src).contains(&"L0006".to_string()));
    }

    #[test]
    fn known_codes_contains_l0006() {
        assert!(
            KNOWN_CODES.contains(&"L0006"),
            "L0006 missing from KNOWN_CODES"
        );
    }

    #[test]
    fn known_codes_contains_l0007() {
        assert!(
            KNOWN_CODES.contains(&"L0007"),
            "L0007 missing from KNOWN_CODES"
        );
    }

    // ---------- L0007: unreachable code after return ----------

    #[test]
    fn l0007_fires_on_stmt_after_return() {
        // Two statements follow the return; only the first is flagged.
        let src = "fn f(int x) {\n    return x;\n    let a = 1;\n    let b = 2;\n}\n";
        let hits: Vec<_> = lint(src)
            .into_iter()
            .filter(|l| l.code == "L0007")
            .collect();
        assert_eq!(hits.len(), 1, "expected exactly one L0007 warning");
        assert_eq!(
            hits[0].line, 3,
            "warning should point to the first unreachable statement"
        );
    }

    #[test]
    fn l0007_silent_on_normal_flow() {
        let src = "fn f(int x) {\n    let a = x + 1;\n    return a;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_silent_when_return_is_last() {
        let src = "fn f() {\n    return;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_suppressed_by_allow_comment() {
        // The allow comment goes on the line above the first unreachable statement.
        let src =
            "fn f(int x) {\n    return x;\n    // resilient: allow L0007\n    let a = 1;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    #[test]
    fn l0007_does_not_fire_for_return_inside_nested_block() {
        // A `return` inside an `if` branch does not make code after the `if` unreachable.
        let src = "fn f(int x) {\n    if x > 0 {\n        return x;\n    }\n    return 0;\n}\n";
        assert!(!codes(src).contains(&"L0007".to_string()));
    }

    // ---------- L0008: duplicate identical struct literal match arm (RES-369) ----------

    #[test]
    fn l0008_fires_on_duplicate_struct_literal_arm() {
        // Two arms with the same struct + same literal field values — the
        // second can never fire.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 0, y: 0 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { x: 0, y: 0 } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0008".to_string()),
            "L0008 must fire when two arms have identical struct literal patterns; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0008_silent_when_arms_differ() {
        // Two arms with the same struct but different field values do not
        // overlap.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 1, y: 2 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { x: 1, y: 1 } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0008".to_string()),
            "L0008 must not fire when struct literal arms have different field values"
        );
    }

    #[test]
    fn l0008_silent_for_rest_pattern() {
        // `Point { .. }` is a wildcard, not a duplicate literal pattern —
        // two `Point { .. }` arms do NOT trigger L0008; the second is
        // caught by L0002 (arm after catch-all) instead.
        let src = "\
            struct Point { int x, int y }\n\
            fn f(int _d) -> int {\n\
                let p = new Point { x: 0, y: 0 };\n\
                return match p {\n\
                    Point { x: 0, y: 0 } => 1,\n\
                    Point { .. } => 2,\n\
                    Point { .. } => 3,\n\
                };\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0008".to_string()),
            "L0008 must not fire for rest/wildcard struct arms"
        );
    }

    #[test]
    fn known_codes_contains_l0008() {
        assert!(
            KNOWN_CODES.contains(&"L0008"),
            "L0008 missing from KNOWN_CODES"
        );
    }

    // ---------- RES-237: L0001 false-positives for Assume / MapLiteral /
    // SetLiteral / LetDestructureStruct ----------

    #[test]
    fn l0001_no_false_positive_in_assume_condition() {
        // `x` is read inside assume() — must not fire L0001.
        let src = "fn f(int x) {\n    let y = x + 1;\n    assume(y > 0);\n    return y;\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when local is used inside assume()"
        );
    }

    #[test]
    fn l0001_no_false_positive_in_map_literal_key() {
        // `key` is a let binding that is used only as a map key.
        // Before RES-237 this fired a false L0001 because MapLiteral
        // was not visited by collect_identifier_reads_in.
        let src = "fn f(int n) -> Int {\n    let key = n + 1;\n    let m = {key -> 0};\n    return map_len(m);\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when let binding is used as a map key"
        );
    }

    #[test]
    fn l0001_no_false_positive_in_set_literal_item() {
        // `elem` is a let binding that is used only inside a set literal.
        // Before RES-237 this fired a false L0001 because SetLiteral
        // was not visited by collect_identifier_reads_in.
        let src = "fn f(int n) -> Int {\n    let elem = n + 1;\n    let s = #{elem};\n    return set_len(s);\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when let binding is used inside a set literal"
        );
    }

    #[test]
    fn l0001_fires_for_unused_struct_destructure_binding() {
        // `b` is bound by destructure but never read.
        let src = "\
            struct Pt { int x, int y }\n\
            fn f(int d) -> Int {\n\
                let p = new Pt { x: 1, y: 2 };\n\
                let Pt { x: a, y: b } = p;\n\
                return a;\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for unused struct-destructure binding"
        );
    }

    #[test]
    fn l0001_silent_for_used_struct_destructure_binding() {
        // Both `a` and `b` are read after destructuring.
        let src = "\
            struct Pt { int x, int y }\n\
            fn f(int d) -> Int {\n\
                let p = new Pt { x: 3, y: 4 };\n\
                let Pt { x: a, y: b } = p;\n\
                return a + b;\n\
            }\n\
        ";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when all struct-destructure bindings are used"
        );
    }

    // ---------- RES-239: lint passes walk impl block methods ----------

    #[test]
    fn l0001_fires_for_unused_binding_in_impl_method() {
        // `unused` is declared but never read inside a method body.
        let src = "\
            struct Counter { int n }\n\
            impl Counter {\n\
                fn tick(self) -> int {\n\
                    let unused = 99;\n\
                    return self.n;\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for unused binding inside an impl method"
        );
    }

    // ---------- RES-259: L0001 fires on unused match-arm bindings ----------

    #[test]
    fn l0001_fires_on_unused_match_arm_binding() {
        // `y` is bound by the pattern but never used in the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        y => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire when a match-arm pattern binding is never used"
        );
    }

    #[test]
    fn l0001_silent_when_match_arm_binding_is_used() {
        // `y` is bound and then returned from the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        y => y,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when a match-arm pattern binding is used"
        );
    }

    #[test]
    fn l0001_silent_for_underscore_prefixed_match_arm_binding() {
        // `_y` starts with `_` — explicitly silenced per convention.
        let src = "fn f(int x) -> int {\n    return match x {\n        _y => 1,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire for underscore-prefixed match-arm binding"
        );
    }

    #[test]
    fn l0001_fires_on_unused_bind_pattern_name() {
        // `n @ _`: `n` is bound but never used in the arm body.
        let src = "fn f(int x) -> int {\n    return match x {\n        n @ _ => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire when the name in a bind pattern (name @ inner) is unused"
        );
    }

    #[test]
    fn l0002_fires_for_unreachable_arm_in_impl_method() {
        // An arm after `_` inside a method is unreachable.
        let src = "\
            struct Wrapper { int v }\n\
            impl Wrapper {\n\
                fn kind(self) -> int {\n\
                    return match self.v {\n\
                        _ => 0,\n\
                        1 => 1,\n\
                    };\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0002".to_string()),
            "L0002 must fire for unreachable arm inside an impl method"
        );
    }

    // ---------- RES-350: L0009 integer division by zero ----------

    #[test]
    fn l0009_fires_on_literal_integer_divisor() {
        // The non-Z3 baseline: literal 0 always fires.
        let src = "fn f(int a) -> int {\n    return a / 0;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire on literal-zero integer divisor; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0009_fires_on_literal_modulo_divisor() {
        let src = "fn f(int a) -> int {\n    return a % 0;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire on literal-zero modulo divisor"
        );
    }

    #[test]
    fn l0009_silent_on_literal_nonzero_divisor() {
        let src = "fn f(int a) -> int {\n    return a / 2;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must not fire on literal-nonzero divisor"
        );
    }

    #[test]
    #[cfg(not(feature = "z3"))]
    fn l0009_silent_on_identifier_divisor_without_z3() {
        // Without Z3, identifier divisors are silent — we only
        // flag statically-obvious literal-zero bugs.
        let src = "fn f(int a, int b) -> int {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent on identifier divisors without the z3 feature"
        );
    }

    #[test]
    fn l0009_suppressed_by_allow_comment() {
        let src = "fn f(int a) -> int {\n    // resilient: allow L0009\n    return a / 0;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must be suppressed by allow comment"
        );
    }

    #[test]
    fn known_codes_contains_l0009() {
        assert!(
            KNOWN_CODES.contains(&"L0009"),
            "L0009 missing from KNOWN_CODES"
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_fires_on_unconstrained_identifier_divisor_with_z3() {
        // Under the z3 feature, a divisor with no precondition is
        // flagged because the solver cannot prove it non-zero.
        let src = "fn f(int a, int b) -> int {\n    return a / b;\n}\n";
        assert!(
            codes(src).contains(&"L0009".to_string()),
            "L0009 must fire when z3 cannot prove divisor non-zero"
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_silent_when_precondition_guarantees_nonzero() {
        // `requires b != 0;` gives Z3 enough to prove the obligation.
        let src = "fn f(int a, int b) -> int requires b != 0 {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent when preconditions prove divisor non-zero; got {:?}",
            codes(src)
        );
    }

    #[test]
    #[cfg(feature = "z3")]
    fn l0009_silent_when_precondition_forces_strictly_positive() {
        // `requires b > 0;` also implies `b != 0`.
        let src = "fn f(int a, int b) -> int requires b > 0 {\n    return a / b;\n}\n";
        assert!(
            !codes(src).contains(&"L0009".to_string()),
            "L0009 must stay silent when b > 0 implies b != 0"
        );
    }

    #[test]
    fn l0005_fires_for_trailing_return_in_impl_method() {
        // A trailing bare `return;` inside a method is redundant.
        let src = "\
            struct Noop { int x }\n\
            impl Noop {\n\
                fn run(self) {\n\
                    let _v = self.x;\n\
                    return;\n\
                }\n\
            }\n\
        ";
        assert!(
            codes(src).contains(&"L0005".to_string()),
            "L0005 must fire for trailing bare return inside an impl method"
        );
    }

    // ---------- L0010: no requires/ensures contract ----------

    #[test]
    fn l0010_fires_on_fn_with_no_contract() {
        let src = "fn f(int x) { return x; }\n";
        assert!(
            codes(src).contains(&"L0010".to_string()),
            "L0010 must fire when a function has no requires/ensures contract"
        );
    }

    #[test]
    fn l0010_silent_when_requires_present() {
        let src = "fn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must stay silent when function has a requires clause"
        );
    }

    #[test]
    fn l0010_silent_when_ensures_present() {
        let src = "fn f(int x) -> int ensures result >= 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must stay silent when function has an ensures clause"
        );
    }

    #[test]
    fn l0010_silent_for_underscore_prefixed_fns() {
        let src = "fn _helper(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must not fire on underscore-prefixed function names"
        );
    }

    #[test]
    fn l0010_allow_comment_suppresses() {
        let src = "// resilient: allow L0010\nfn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0010".to_string()),
            "L0010 must be suppressible with allow comment"
        );
    }

    #[test]
    fn l0010_in_known_codes() {
        assert!(
            KNOWN_CODES.contains(&"L0010"),
            "L0010 must appear in KNOWN_CODES"
        );
    }

    // ---------- RES-308 / L0011: unused variable warning ----------

    #[test]
    fn l0011_fires_on_unused_let() {
        // `unused` is bound and never read — must produce L0011.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        assert!(
            codes(src).contains(&"L0011".to_string()),
            "L0011 must fire on a `let` binding whose name is never read"
        );
    }

    #[test]
    fn l0011_silent_when_let_is_used() {
        // Used `let` — no L0011.
        let src = "fn f(int a) {\n    let used = a + 1;\n    return used;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must not fire when the let binding is read"
        );
    }

    #[test]
    fn l0011_silent_for_underscore_prefix() {
        // Underscore-prefixed names are exempt by convention.
        let src = "fn f(int a) {\n    let _temp = 42;\n    return a;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must not fire for `_`-prefixed bindings"
        );
    }

    #[test]
    fn l0011_message_matches_ticket_format() {
        // RES-308 specifies the exact rustc-style phrasing.
        let src = "fn f(int a) {\n    let zzz = 42;\n    return a;\n}\n";
        let lints = lint(src);
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(
            l0011.message, "variable `zzz` is assigned but never used",
            "L0011 message must match the RES-308 acceptance criteria"
        );
        assert_eq!(l0011.severity, Severity::Warning);
    }

    #[test]
    fn l0011_reports_at_let_span() {
        // `let unused = 42;` is on line 2, indent 4 — column 5.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        let lints = lint(src);
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(l0011.line, 2, "L0011 must report at the let line");
    }

    #[test]
    fn l0011_suppressed_by_allow_comment() {
        let src = "fn f(int a) {\n    // resilient: allow L0011\n    let unused = 42;\n    return a;\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must be suppressible with `// resilient: allow L0011`"
        );
    }

    #[test]
    fn l0011_in_known_codes() {
        // The CLI validates --deny / --allow against KNOWN_CODES;
        // missing L0011 here would silently reject `--deny L0011`.
        assert!(
            KNOWN_CODES.contains(&"L0011"),
            "L0011 must appear in KNOWN_CODES so --deny/--allow accept it"
        );
    }

    #[test]
    fn l0011_deny_escalates_to_error() {
        // Mirrors the L0001 escalation path — `--deny L0011` should
        // bump severity to Error. This unit test simulates the flag
        // by mutating severity directly; the `lint_smoke.rs`
        // integration test exercises the CLI plumbing end-to-end.
        let src = "fn f(int a) {\n    let unused = 42;\n    return a;\n}\n";
        let mut lints = lint(src);
        for l in lints.iter_mut() {
            if l.code == "L0011" {
                l.severity = Severity::Error;
            }
        }
        let l0011 = lints
            .iter()
            .find(|l| l.code == "L0011")
            .expect("expected L0011 to fire");
        assert_eq!(
            l0011.severity,
            Severity::Error,
            "L0011 must escalate to Error under --deny L0011"
        );
    }

    #[test]
    fn l0011_fires_inside_impl_method() {
        // Same as RES-239 coverage for L0001 — impl-block methods
        // must be walked.
        let src = "struct S {}\nimpl S {\n    fn m(self) {\n        let unused = 42;\n        return;\n    }\n}\n";
        assert!(
            codes(src).contains(&"L0011".to_string()),
            "L0011 must fire for unused let inside an impl method"
        );
    }

    #[test]
    fn l0011_silent_when_used_inside_live_block() {
        // The ticket explicitly notes that vars used only inside a
        // `live` block retry path are NOT exempt — but a var that
        // IS read inside a `live` body must NOT fire L0011.
        let src = "fn f() {\n    let x = 1;\n    live { return x; }\n}\n";
        assert!(
            !codes(src).contains(&"L0011".to_string()),
            "L0011 must treat reads inside a `live` body as uses"
        );
    }

    // ---------- RES-397 / L0012: spec provenance ----------

    #[test]
    fn l0012_in_known_codes() {
        assert!(
            KNOWN_CODES.contains(&"L0012"),
            "L0012 must appear in KNOWN_CODES so --deny/--allow accept it"
        );
    }

    #[test]
    fn l0012_fires_on_function_with_requires_but_no_source() {
        let src = "fn f(int x) requires x > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with requires but no `// source:` comment"
        );
    }

    #[test]
    fn l0012_fires_on_function_with_ensures_but_no_source() {
        let src = "fn f(int x) ensures result > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with ensures but no `// source:` comment"
        );
    }

    #[test]
    fn l0012_silent_when_source_comment_present() {
        let src = "// source: RFC 9293 §3.5\nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire when `// source:` precedes the spec-bearing fn"
        );
    }

    #[test]
    fn l0012_silent_for_function_without_spec() {
        // A fn with neither requires nor ensures is L0010's territory,
        // not L0012's. L0012 only fires when there IS a spec.
        let src = "fn f(int x) { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on a fn without any spec annotation"
        );
    }

    #[test]
    fn l0012_silent_for_underscore_prefixed_function() {
        let src = "fn _helper(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on `_`-prefixed helper fns"
        );
    }

    #[test]
    fn l0012_fires_on_assume_without_source() {
        let src = "fn f(int x) {\n    assume(x > 0);\n    return x;\n}\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on `assume()` without a preceding `// source:` comment"
        );
    }

    #[test]
    fn l0012_silent_when_source_comment_precedes_assume() {
        let src = "fn f(int x) {\n    // source: derived from caller's domain\n    assume(x > 0);\n    return x;\n}\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must not fire on `assume()` with a `// source:` line above"
        );
    }

    #[test]
    fn l0012_suppressed_by_allow_comment() {
        let src = "// resilient: allow L0012\nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            !codes(src).contains(&"L0012".to_string()),
            "L0012 must be suppressible with `// resilient: allow L0012`"
        );
    }

    #[test]
    fn l0012_empty_source_comment_does_not_satisfy() {
        // `// source:` with nothing after the colon must not
        // count — the whole point is to require a real reference.
        let src = "// source:   \nfn f(int x) requires x > 0 { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must still fire when `// source:` is empty"
        );
    }

    #[test]
    fn l0012_fires_on_recovers_to() {
        // RES-387: fns with `fails` + `recovers_to:` are spec-bearing too.
        let src = "fn f(int x) fails Bad recovers_to: x > 0; { return x; }\n";
        assert!(
            codes(src).contains(&"L0012".to_string()),
            "L0012 must fire on a fn with `recovers_to:` but no `// source:` comment"
        );
    }

    // ---- L0014 tests ----

    #[test]
    fn l0014_defined_but_never_called() {
        let src = "fn helper(int x) -> int { return x; }\nfn main() { let _y = 1; }\nmain();\n";
        assert!(
            codes(src).contains(&"L0014".to_string()),
            "L0014 must fire for `helper` which is defined but never called"
        );
    }

    #[test]
    fn l0014_called_function_not_flagged() {
        let src =
            "fn helper(int x) -> int { return x; }\nfn main() { let _y = helper(1); }\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire when the function is actually called"
        );
    }

    #[test]
    fn l0014_underscore_prefix_not_flagged() {
        let src = "fn _unused(int x) -> int { return x; }\nfn main() {}\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire for `_`-prefixed functions"
        );
    }

    #[test]
    fn l0014_main_not_flagged_even_if_only_at_top_level() {
        // `main` called at top level (as a statement) should not be flagged.
        let src = "fn main() { let _x = 1; }\nmain();\n";
        assert!(
            !codes(src).contains(&"L0014".to_string()),
            "L0014 must not fire for `main` when called at top level"
        );
    }

    // ---- L0015: constant integer overflow ----

    #[test]
    fn l0015_fires_on_addition_overflow() {
        // 9223372036854775807 + 1 overflows i64.
        let src = "fn f() -> int { return 9223372036854775807 + 1; }\nf();\n";
        assert!(
            codes(src).contains(&"L0015".to_string()),
            "L0015 must fire when literal addition overflows i64; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0015_fires_on_multiplication_overflow() {
        let src = "fn f() -> int { return 1000000000 * 1000000000000000; }\nf();\n";
        assert!(
            codes(src).contains(&"L0015".to_string()),
            "L0015 must fire on multiplication overflow"
        );
    }

    #[test]
    fn l0015_silent_for_non_overflowing_expression() {
        let src = "fn f() -> int { return 100 + 200; }\nf();\n";
        assert!(
            !codes(src).contains(&"L0015".to_string()),
            "L0015 must not fire for non-overflowing constant arithmetic"
        );
    }

    #[test]
    fn l0015_silent_when_operand_is_variable() {
        // `x + 1` — not fully constant, so overflow cannot be proven.
        let src = "fn f(int x) -> int { return x + 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0015".to_string()),
            "L0015 must not fire when an operand is a variable"
        );
    }

    // ---- L0016: constant boolean condition ----

    #[test]
    fn l0016_fires_on_literal_true_condition() {
        let src = "fn f() { if true { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when `if` condition is literal `true`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0016_fires_on_literal_false_condition() {
        let src = "fn f() { if false { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when `if` condition is literal `false`"
        );
    }

    #[test]
    fn l0016_fires_on_constant_comparison() {
        // `1 < 2` is always true — equivalent to a literal `true`.
        let src = "fn f() { if 1 < 2 { let _x = 1; } }\nf();\n";
        assert!(
            codes(src).contains(&"L0016".to_string()),
            "L0016 must fire when condition folds to a constant bool"
        );
    }

    #[test]
    fn l0016_silent_for_variable_condition() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } return 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0016".to_string()),
            "L0016 must not fire when condition involves a variable"
        );
    }

    // ---- L0017: variable shadowing ----

    #[test]
    fn l0017_fires_when_let_shadows_outer_let() {
        // Inner `let x` shadows outer `let x`.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let x = n + 1;\n        return x;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0017".to_string()),
            "L0017 must fire when inner let shadows outer let; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0017_fires_when_let_shadows_parameter() {
        // `let n` shadows parameter `n`.
        let src = "fn f(int n) -> int {\n    let n = n + 1;\n    return n;\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0017".to_string()),
            "L0017 must fire when let shadows a parameter"
        );
    }

    #[test]
    fn l0017_silent_for_underscore_prefix() {
        // `_x` is exempt — underscore prefix signals intentional shadowing.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let _x = n + 1;\n        return _x;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0017".to_string()),
            "L0017 must not fire for `_`-prefixed bindings"
        );
    }

    #[test]
    fn l0017_silent_when_no_shadowing() {
        // `y` is a new name, not a shadow.
        let src = "fn f(int n) -> int {\n    let x = n;\n    if n > 0 {\n        let y = n + 1;\n        return y;\n    }\n    return x;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0017".to_string()),
            "L0017 must not fire when names are distinct"
        );
    }

    // ---- L0018: missing return on all paths ----

    #[test]
    fn l0018_fires_when_if_without_else_is_last() {
        // Return type is `int` but the if-without-else path falls through.
        let src = "fn f(int x) -> int {\n    if x > 0 { return 1; }\n}\nf(1);\n";
        assert!(
            codes(src).contains(&"L0018".to_string()),
            "L0018 must fire when fn with return type lacks an else branch; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0018_silent_when_all_paths_return() {
        // Both branches of the if/else return, so all paths are covered.
        let src = "fn f(int x) -> int {\n    if x > 0 { return 1; } else { return 0; }\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire when every path ends with return"
        );
    }

    #[test]
    fn l0018_silent_for_void_function() {
        // No return type annotation — void function, L0018 does not apply.
        let src = "fn f(int x) {\n    if x > 0 { let _y = 1; }\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire for functions with no return type"
        );
    }

    #[test]
    fn l0018_silent_when_return_at_end_of_body() {
        // Unconditional return at end of body covers all paths.
        let src = "fn f(int x) -> int {\n    let y = x + 1;\n    return y;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0018".to_string()),
            "L0018 must not fire when body ends with an unconditional return"
        );
    }

    // ---- L0019: format() arity mismatch ----

    #[test]
    fn l0019_fires_on_missing_args_array() {
        // format() called with only 1 argument (missing args array).
        let src = "fn f() { let _s = format(\"hello\"); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when format() has only 1 argument; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0019_fires_on_too_many_toplevel_args() {
        // format() called with 3 arguments instead of 2.
        let src = "fn f() { let _s = format(\"hello\", [], []); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when format() has 3+ arguments"
        );
    }

    #[test]
    fn l0019_fires_on_placeholder_array_mismatch() {
        // Template `\{} \{}` has 2 placeholders but array has 1 element.
        // Rust string "\{} \{}" encodes the Resilient source `\{} \{}`.
        let src = "fn f() { let _s = format(\"\\{} \\{}\", [1]); }\nf();\n";
        assert!(
            codes(src).contains(&"L0019".to_string()),
            "L0019 must fire when placeholder count != array length; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0019_silent_on_exact_placeholder_match() {
        // Template `\{}` has 1 placeholder and array has 1 element.
        let src = "fn f() { let _s = format(\"\\{}\", [42]); }\nf();\n";
        assert!(
            !codes(src).contains(&"L0019".to_string()),
            "L0019 must not fire when placeholder count matches array length"
        );
    }

    #[test]
    fn l0019_silent_for_no_placeholders_empty_array() {
        // Plain string with no placeholders, empty args array — clean.
        let src = "fn f() { let _s = format(\"hello world\", []); }\nf();\n";
        assert!(
            !codes(src).contains(&"L0019".to_string()),
            "L0019 must not fire for template with no placeholders and empty array"
        );
    }

    // ---------- L0020: unused function parameter ----------

    #[test]
    fn l0020_fires_on_unused_parameter() {
        let src = "fn f(int a, int b) -> int { return a; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0020".to_string()),
            "L0020 must fire when parameter `b` is never used"
        );
    }

    #[test]
    fn l0020_silent_when_all_params_used() {
        let src = "// source: test\nfn f(int a, int b) -> int requires a > 0 && b > 0 { return a + b; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire when all parameters are used"
        );
    }

    #[test]
    fn l0020_silent_for_underscore_prefix() {
        let src = "// source: test\nfn f(int a, int _unused) -> int requires a > 0 { return a; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire for `_`-prefixed parameters"
        );
    }

    #[test]
    fn l0020_silent_when_param_used_only_in_requires() {
        // Parameter only in `requires` clause counts as used.
        let src =
            "// source: test\nfn f(int a, int b) -> int requires b > 0 { return a; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0020".to_string()),
            "L0020 must not fire when parameter used in requires clause"
        );
    }

    // ---------- L0021: redundant boolean sub-expression ----------

    #[test]
    fn l0021_fires_on_x_and_x() {
        let src = "fn f(bool x) -> bool { return x && x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0021".to_string()),
            "L0021 must fire for `x && x`"
        );
    }

    #[test]
    fn l0021_fires_on_x_or_x() {
        let src = "fn f(bool x) -> bool { return x || x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0021".to_string()),
            "L0021 must fire for `x || x`"
        );
    }

    #[test]
    fn l0021_silent_for_distinct_operands() {
        let src = "// source: test\nfn f(bool x, bool y) -> bool requires true { return x && y; }\nf(true, false);\n";
        assert!(
            !codes(src).contains(&"L0021".to_string()),
            "L0021 must not fire when operands differ"
        );
    }

    // ---------- L0022: needless else after return ----------

    #[test]
    fn l0022_fires_on_else_after_return() {
        let src = "fn f(int x) -> int { if x > 0 { return 1; } else { return 0; } }\nf(1);\n";
        assert!(
            codes(src).contains(&"L0022".to_string()),
            "L0022 must fire when if-consequence always returns and else is present"
        );
    }

    #[test]
    fn l0022_silent_when_consequence_may_fall_through() {
        // Consequence doesn't always return (loop without return).
        let src = "// source: test\nfn f(int x) -> int requires x > 0 { if x > 0 { let _y = 1; } else { return 0; } return 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0022".to_string()),
            "L0022 must not fire when consequence doesn't always return"
        );
    }

    #[test]
    fn l0022_silent_when_no_else() {
        let src = "// source: test\nfn f(int x) -> int requires x > 0 { if x > 0 { return 1; } return 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0022".to_string()),
            "L0022 must not fire when there is no else branch"
        );
    }

    // ---------- L0023: tautological comparison with boolean literal ----------

    #[test]
    fn l0023_fires_on_eq_true() {
        let src = "fn f(bool x) -> bool { return x == true; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire for `x == true`"
        );
    }

    #[test]
    fn l0023_fires_on_eq_false() {
        let src = "fn f(bool x) -> bool { return x == false; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire for `x == false`"
        );
    }

    #[test]
    fn l0023_fires_on_literal_left() {
        let src = "fn f(bool x) -> bool { return true == x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0023".to_string()),
            "L0023 must fire when literal is on the left side"
        );
    }

    #[test]
    fn l0023_silent_for_non_bool_comparison() {
        let src = "// source: test\nfn f(int x) -> bool requires x > 0 { return x == 1; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0023".to_string()),
            "L0023 must not fire for integer comparisons"
        );
    }

    // ---------- L0025: unreachable code after infinite while-true loop ----------

    #[test]
    fn l0025_fires_on_code_after_while_true() {
        let src =
            "fn f() {\n    while true {\n        let _x = 1;\n    }\n    let dead = 2;\n}\nf();\n";
        assert!(
            codes(src).contains(&"L0025".to_string()),
            "L0025 must fire when code follows while-true with no break"
        );
    }

    #[test]
    fn l0025_silent_when_loop_has_break() {
        let src = "// source: test\nfn f() -> int requires true {\n    while true {\n        break;\n    }\n    return 0;\n}\nf();\n";
        assert!(
            !codes(src).contains(&"L0025".to_string()),
            "L0025 must not fire when while-true loop contains a break"
        );
    }

    #[test]
    fn l0025_silent_for_conditional_while() {
        let src = "// source: test\nfn f(int x) -> int requires x > 0 {\n    while x > 0 {\n        let _y = 1;\n    }\n    return 0;\n}\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0025".to_string()),
            "L0025 must not fire for non-literal while condition"
        );
    }

    // ---------- L0024: struct literal missing required fields ----------

    #[test]
    fn l0024_fires_when_field_missing() {
        let src = "struct Point { int x, int y, int z }\nlet _p = new Point { x: 1, y: 2 };\n";
        assert!(
            codes(src).contains(&"L0024".to_string()),
            "L0024 must fire when a struct literal omits a declared field; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0024_silent_when_all_fields_present() {
        let src = "struct Point { int x, int y }\nlet _p = new Point { x: 1, y: 2 };\n";
        assert!(
            !codes(src).contains(&"L0024".to_string()),
            "L0024 must not fire when all declared fields are provided"
        );
    }

    #[test]
    fn l0024_message_names_missing_fields() {
        let src = "struct Rect { int w, int h, int depth }\nlet _r = new Rect { w: 10 };\n";
        let lints = lint(src);
        let l = lints.iter().find(|l| l.code == "L0024");
        assert!(l.is_some(), "L0024 must fire; got {:?}", lints);
        let msg = &l.unwrap().message;
        assert!(
            msg.contains("`h`") && msg.contains("`depth`"),
            "L0024 message must name missing fields; got: {msg}"
        );
    }

    #[test]
    fn l0024_silent_for_unknown_struct_name() {
        // If the struct isn't declared in this program, don't fire.
        let src = "let _p = new Unknown { x: 1 };\n";
        assert!(
            !codes(src).contains(&"L0024".to_string()),
            "L0024 must not fire for unknown struct type"
        );
    }

    // ---------- L0026: duplicate key in map literal ----------

    #[test]
    fn l0026_fires_on_duplicate_string_key() {
        let src = "fn f() { let _m = {\"a\" -> 1, \"b\" -> 2, \"a\" -> 3}; }\nf();\n";
        assert!(
            codes(src).contains(&"L0026".to_string()),
            "L0026 must fire when a string key appears twice; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0026_fires_on_duplicate_integer_key() {
        let src = "fn f() { let _m = {1 -> \"x\", 2 -> \"y\", 1 -> \"z\"}; }\nf();\n";
        assert!(
            codes(src).contains(&"L0026".to_string()),
            "L0026 must fire when an integer key appears twice"
        );
    }

    #[test]
    fn l0026_silent_when_keys_unique() {
        let src = "fn f() { let _m = {\"a\" -> 1, \"b\" -> 2, \"c\" -> 3}; }\nf();\n";
        assert!(
            !codes(src).contains(&"L0026".to_string()),
            "L0026 must not fire when all keys are distinct"
        );
    }

    // ---------- L0027: empty catch block ----------

    #[test]
    fn l0027_fires_on_empty_catch() {
        let src = "fn risky() fails Bad { }\ntry { risky(); } catch Bad { }\n";
        assert!(
            codes(src).contains(&"L0027".to_string()),
            "L0027 must fire for an empty catch block; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0027_silent_when_catch_has_body() {
        let src = "fn risky() fails Bad { }\ntry { risky(); } catch Bad { let _x = 1; }\n";
        assert!(
            !codes(src).contains(&"L0027".to_string()),
            "L0027 must not fire when the catch block has statements"
        );
    }

    // ---------- L0028: negation of boolean literal ----------

    #[test]
    fn l0028_fires_on_not_true() {
        let src = "let _x = !true;\n";
        assert!(
            codes(src).contains(&"L0028".to_string()),
            "L0028 must fire for `!true`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0028_fires_on_not_false() {
        let src = "let _x = !false;\n";
        assert!(
            codes(src).contains(&"L0028".to_string()),
            "L0028 must fire for `!false`"
        );
    }

    #[test]
    fn l0028_silent_for_not_identifier() {
        let src = "fn f(bool x) -> bool { return !x; }\nf(true);\n";
        assert!(
            !codes(src).contains(&"L0028".to_string()),
            "L0028 must not fire for `!identifier`"
        );
    }

    // ---------- L0029: comparison result discarded ----------

    #[test]
    fn l0029_fires_on_discarded_eq() {
        let src = "fn f(int x, int y) { x == y; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0029".to_string()),
            "L0029 must fire when comparison result is discarded; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0029_fires_on_discarded_lt() {
        let src = "fn f(int x, int y) { x < y; }\nf(1, 2);\n";
        assert!(
            codes(src).contains(&"L0029".to_string()),
            "L0029 must fire when `<` result is discarded"
        );
    }

    #[test]
    fn l0029_silent_when_used_in_if() {
        let src =
            "fn f(int x, int y) -> bool { if x == y { return true; } return false; }\nf(1, 2);\n";
        assert!(
            !codes(src).contains(&"L0029".to_string()),
            "L0029 must not fire when comparison is used as condition"
        );
    }

    // ---------- L0030: float equality comparison ----------

    #[test]
    fn l0030_fires_on_float_eq_zero() {
        let src = "fn f(float x) -> bool { return x == 0.0; }\nf(1.0);\n";
        assert!(
            codes(src).contains(&"L0030".to_string()),
            "L0030 must fire for float == literal; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0030_fires_on_float_neq() {
        let src = "fn f(float x) -> bool { return 1.5 != x; }\nf(1.0);\n";
        assert!(
            codes(src).contains(&"L0030".to_string()),
            "L0030 must fire for float literal != expression"
        );
    }

    #[test]
    fn l0030_silent_for_int_equality() {
        let src = "fn f(int x) -> bool { return x == 0; }\nf(1);\n";
        assert!(
            !codes(src).contains(&"L0030".to_string()),
            "L0030 must not fire for integer equality"
        );
    }

    // ---------- L0031: double negation ----------

    #[test]
    fn l0031_fires_on_double_not() {
        let src = "fn f(bool x) -> bool { return !!x; }\nf(true);\n";
        assert!(
            codes(src).contains(&"L0031".to_string()),
            "L0031 must fire for `!!x`; got {:?}",
            codes(src)
        );
    }

    #[test]
    fn l0031_silent_for_single_not() {
        let src = "fn f(bool x) -> bool { return !x; }\nf(true);\n";
        assert!(
            !codes(src).contains(&"L0031".to_string()),
            "L0031 must not fire for a single negation"
        );
    }
}
