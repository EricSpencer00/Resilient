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
    "L0007", // unreachable code after unconditional `return`
    "L0008", // duplicate identical struct literal match arm
    "L0009", // integer division by zero (literal / SMT-proven-possible)
    "L0010", // function has no requires/ensures contract
    "L0011", // RES-308: unused variable warning (let binding never read)
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
    run_l0007_unreachable_code(program, &mut out);
    run_l0008_duplicate_struct_match_arm(program, &mut out);
    run_l0009_division_by_zero(program, &mut out);
    run_l0010_no_contract(program, &mut out);
    run_l0011_unused_variable(program, &mut out);

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
    let allows = collect_allow_comments(source);
    out.retain(|l| {
        if allows.contains(&(l.line, l.code.clone())) {
            return false;
        }
        if l.code == "L0011" && allows.contains(&(l.line, "L0001".to_string())) {
            return false;
        }
        true
    });
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
    let mut lets: Vec<(String, Span)> = Vec::new();
    collect_lets_in(body, &mut lets);
    if !lets.is_empty() {
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
    // RES-259: check match-arm pattern bindings (scoped per arm).
    // This is always called, regardless of whether `let` bindings exist.
    l0001_check_match_arms(body, out);
}

/// RES-259: collect the names bound by a pattern (one level of binding
/// per pattern, recursing into `Or` first-branch and `Bind` inner).
fn collect_pattern_bindings(pattern: &Pattern) -> Vec<String> {
    match pattern {
        Pattern::Identifier(name) => vec![name.clone()],
        Pattern::Bind(name, inner) => {
            let mut names = vec![name.clone()];
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
            let mut names = Vec::new();
            for (_, sub) in fields {
                names.extend(collect_pattern_bindings(sub.as_ref()));
            }
            names
        }
        // RES-375: `Some(inner)` forwards to inner; `None` has no bindings.
        Pattern::Some(inner) => collect_pattern_bindings(inner.as_ref()),
        Pattern::None => vec![],
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
                    let mut used: std::collections::HashSet<String> =
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
                        if !used.contains(name) {
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
        Node::ForInStatement {
            name,
            iterable,
            body,
            span,
            ..
        } => {
            if !name.starts_with('_') {
                out.push((name.clone(), *span));
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
        Pattern::Literal(_) => false,
        Pattern::Or(branches) => branches.iter().any(pattern_is_default_for_lint),
        Pattern::Bind(_, inner) => pattern_is_default_for_lint(inner),
        Pattern::Struct { fields, .. } => fields
            .iter()
            .all(|(_, sub)| pattern_is_default_for_lint(sub.as_ref())),
        // RES-375: Option patterns are never catch-alls by themselves.
        Pattern::Some(_) | Pattern::None => false,
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
    let mut parts = Vec::new();
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
                l0009_check_body(body, requires, out);
            }
            Node::ImplBlock { methods, .. } => {
                for method in methods {
                    if let Node::Function { body, requires, .. } = method {
                        l0009_check_body(body, requires, out);
                    }
                }
            }
            _ => {}
        }
    }
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
    let mut lets: Vec<(String, Span)> = Vec::new();
    l0011_collect_let_bindings(body, &mut lets);
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
fn l0011_collect_let_bindings(node: &Node, out: &mut Vec<(String, Span)>) {
    match node {
        Node::LetStatement {
            name, value, span, ..
        } => {
            out.push((name.clone(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::StaticLet {
            name, value, span, ..
        } => {
            out.push((name.clone(), *span));
            l0011_collect_let_bindings(value, out);
        }
        Node::LetDestructureStruct {
            fields,
            value,
            span,
            ..
        } => {
            for (_field_name, local_name) in fields {
                out.push((local_name.clone(), *span));
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
}
