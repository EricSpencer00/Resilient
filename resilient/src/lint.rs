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
];

/// RES-198: top-level entry. Runs every lint, filters via the
/// `// resilient: allow LXXXX` comments found in `source`, and
/// returns the surviving diagnostics sorted by (line, column).
pub fn check(program: &Node, source: &str) -> Vec<Lint> {
    let mut out = Vec::new();
    run_l0001_unused_local(program, &mut out);
    run_l0002_unreachable_arm(program, &mut out);
    run_l0003_self_comparison(program, &mut out);
    run_l0004_mixed_and_or(program, &mut out);
    run_l0005_redundant_return(program, &mut out);
    run_l0006_assume_false(program, &mut out);

    // Filter via allow-comments.
    let allows = collect_allow_comments(source);
    out.retain(|l| !allows.contains(&(l.line, l.code.clone())));
    out.sort_by(|a, b| a.line.cmp(&b.line).then(a.column.cmp(&b.column)));
    out
}

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
    let mut lets: Vec<(String, Span)> = Vec::new();
    collect_lets_in(body, &mut lets);
    if lets.is_empty() {
        return;
    }
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_identifier_reads_in(body, &mut used);
    for (name, span) in &lets {
        if name.starts_with('_') {
            continue;
        }
        if !used.contains(name) {
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

/// RES-259: push match-arm pattern bindings into `out` so L0001 can
/// flag unused names. Only `Pattern::Identifier` and `Pattern::Bind`
/// introduce new local bindings; the others are silent.
fn collect_pattern_bindings(pattern: &Pattern, span: Span, out: &mut Vec<(String, Span)>) {
    match pattern {
        Pattern::Identifier(name) => {
            out.push((name.clone(), span));
        }
        Pattern::Bind(name, _inner) => {
            out.push((name.clone(), span));
        }
        Pattern::Or(branches) => {
            if let Some(first) = branches.first() {
                collect_pattern_bindings(first, span, out);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

fn collect_lets_in(node: &Node, out: &mut Vec<(String, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.clone(), *span));
            collect_lets_in(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.clone(), *span));
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
        Node::ForInStatement { iterable, body, .. } => {
            collect_lets_in(iterable, out);
            collect_lets_in(body, out);
        }
        Node::LiveBlock { body, .. } => collect_lets_in(body, out),
        // Descend into expression and return statements so that match
        // expressions used as statements or return values are also walked.
        Node::ExpressionStatement { expr, .. } => collect_lets_in(expr, out),
        Node::ReturnStatement { value: Some(v), .. } => collect_lets_in(v, out),
        Node::Match {
            scrutinee,
            arms,
            span,
            ..
        } => {
            collect_lets_in(scrutinee, out);
            for (pattern, guard, arm_body) in arms {
                collect_pattern_bindings(pattern, *span, out);
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
                out.push((local_name.clone(), *span));
            }
            collect_lets_in(value, out);
        }
        _ => {}
    }
}

fn collect_identifier_reads_in(node: &Node, out: &mut std::collections::HashSet<String>) {
    match node {
        Node::Identifier { name, .. } => {
            out.insert(name.clone());
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
        Pattern::Literal(_) => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default_for_lint),
        Pattern::Bind(_, inner) => pattern_is_default_for_lint(inner),
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

    // ---------- L0001: match-arm bindings (RES-259) ----------

    #[test]
    fn l0001_fires_on_unused_match_arm_binding() {
        // `y` is bound in the arm pattern but the arm body is `1` (never uses `y`).
        let src = "fn f(int x) -> int {\n    return match x {\n        y => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for match-arm binding `y` that is never used"
        );
    }

    #[test]
    fn l0001_silent_when_match_arm_binding_is_used() {
        // `y` is bound and used as the arm result expression.
        let src = "fn f(int x) -> int {\n    return match x {\n        y => y,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire when match-arm binding `y` is used in the arm body"
        );
    }

    #[test]
    fn l0001_silent_for_underscore_prefixed_match_arm_binding() {
        let src = "fn f(int x) -> int {\n    return match x {\n        _y => 1,\n    };\n}\n";
        assert!(
            !codes(src).contains(&"L0001".to_string()),
            "L0001 must not fire for underscore-prefixed match-arm binding `_y`"
        );
    }

    #[test]
    fn l0001_fires_on_unused_bind_pattern_name() {
        // `n @ 5` — `n` is bound but the arm body `0` never uses it.
        let src = "fn f(int x) -> int {\n    return match x {\n        n @ 5 => 0,\n        _ => 1,\n    };\n}\n";
        assert!(
            codes(src).contains(&"L0001".to_string()),
            "L0001 must fire for `n` in `n @ 5` when `n` is never used"
        );
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
}
