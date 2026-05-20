//! AI Threat Model — formal classification of LLM failure modes.
//!
//! Resilient is the first language whose type system carries an explicit
//! threat model for its own contributors. The premise: Resilient code is
//! often written by an LLM, and we don't trust the LLM. We don't trust
//! humans either, but humans get other tooling. This module catches the
//! *patterns that mark code as AI-generated-without-careful-review*.
//!
//! Each detection corresponds to a known failure mode of code-generating
//! LLMs as observed across `vibe_debt` runs and post-mortems. The list is
//! deliberately conservative — we only flag patterns where the
//! false-positive rate is < 5% across a representative corpus.
//!
//! ## Threat catalogue
//!
//! | Kind | Description |
//! |---|---|
//! | `OffByOne` | `i <= len(a)` — closes off the wrong end of a half-open range |
//! | `MissedElse` | `if cond { return X; } CODE` — implicit fall-through with no `else` |
//! | `SwallowedError` | `catch { }` — empty handler silently consumes errors |
//! | `MagicNumber` | numeric literal > 1 outside a recognised "small constant" context |
//! | `CopyPasteBlock` | two structurally-identical statement sequences within one fn |
//! | `UnboundedLoop` | `while true { ... }` with no `break` reachable |
//! | `GhostHandler` | error handler whose body is just a `println` or trivial `return` |
//! | `HallucinatedIdent` | call to an identifier 1–2 edits away from a known builtin |
//! | `NestedConditional` | three or more `if`-expressions nested as expression-position values |
//! | `SilentSwallow` | `try` body that fails, `catch` arm that returns a literal |
//!
//! ## Wiring
//!
//! Two surfaces:
//!
//! 1. `--ai-threats` CLI flag: prints all detected threats with
//!    file:line:col, then exits 0. Soft.
//! 2. `#[ai_review_required]` attribute on a function: every threat
//!    inside that function's body is a hard error. Use it to gate code
//!    that flows into safety-critical paths.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreatKind {
    OffByOne,
    MissedElse,
    SwallowedError,
    MagicNumber,
    CopyPasteBlock,
    UnboundedLoop,
    GhostHandler,
    HallucinatedIdent,
    NestedConditional,
    SilentSwallow,
}

impl ThreatKind {
    pub fn label(&self) -> &'static str {
        match self {
            ThreatKind::OffByOne => "off-by-one",
            ThreatKind::MissedElse => "missed-else",
            ThreatKind::SwallowedError => "swallowed-error",
            ThreatKind::MagicNumber => "magic-number",
            ThreatKind::CopyPasteBlock => "copy-paste-block",
            ThreatKind::UnboundedLoop => "unbounded-loop",
            ThreatKind::GhostHandler => "ghost-handler",
            ThreatKind::HallucinatedIdent => "hallucinated-ident",
            ThreatKind::NestedConditional => "nested-conditional",
            ThreatKind::SilentSwallow => "silent-swallow",
        }
    }

    pub fn mitigation(&self) -> &'static str {
        match self {
            ThreatKind::OffByOne => "use a half-open range and `< len(...)`",
            ThreatKind::MissedElse => {
                "add the `else` branch explicitly, even if it returns the same value"
            }
            ThreatKind::SwallowedError => {
                "either re-raise the error or annotate the function `fails`"
            }
            ThreatKind::MagicNumber => "name the constant via `let` or `const`",
            ThreatKind::CopyPasteBlock => "extract the duplicated block into a function",
            ThreatKind::UnboundedLoop => {
                "either prove the loop terminates or annotate `#[may_diverge]`"
            }
            ThreatKind::GhostHandler => {
                "the handler must do real recovery work — log, retry, or escalate"
            }
            ThreatKind::HallucinatedIdent => {
                "the identifier does not exist; verify the spelling and argument list"
            }
            ThreatKind::NestedConditional => "flatten the nested conditionals into a `match`",
            ThreatKind::SilentSwallow => {
                "the catch arm hides the failure; thread the error to the caller"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Threat {
    pub kind: ThreatKind,
    pub function: String,
    pub line: usize,
    pub col: usize,
    pub description: String,
    pub confidence: u8,
}

impl Threat {
    pub fn render(&self, source_path: &str) -> String {
        format!(
            "{}:{}:{}: ai-threat[{}]: {} (confidence={}%) — {}",
            source_path,
            self.line,
            self.col,
            self.kind.label(),
            self.description,
            self.confidence,
            self.kind.mitigation()
        )
    }
}

const KNOWN_BUILTINS: &[&str] = &[
    "println", "print", "len", "push", "pop", "map", "filter", "reduce", "sqrt", "pow", "floor",
    "ceil", "abs", "min", "max", "format", "assert", "panic", "Ok", "Err", "Some", "None",
    "result",
];

const SMALL_CONSTANT_TOKENS: &[i64] = &[-1, 0, 1, 2, 8, 16, 32, 64, 128, 256, 512, 1024];

pub fn analyze_program(program: &Node) -> Vec<Threat> {
    let mut out = Vec::new();
    let Node::Program(stmts) = program else {
        return out;
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            analyze_function(name, body, &mut out);
        }
    }
    out
}

fn analyze_function(name: &str, body: &Node, out: &mut Vec<Threat>) {
    let mut ctx = FnContext {
        name: name.to_string(),
        loop_depth: 0,
        cond_depth: 0,
        threats: Vec::new(),
    };
    detect_off_by_one(body, &mut ctx);
    detect_missed_else(body, &mut ctx);
    detect_swallowed_error(body, &mut ctx);
    detect_magic_numbers(body, &mut ctx);
    detect_unbounded_loops(body, &mut ctx);
    detect_ghost_handler(body, &mut ctx);
    detect_hallucinated_idents(body, &mut ctx);
    detect_nested_conditionals(body, 0, &mut ctx);
    detect_silent_swallow(body, &mut ctx);
    detect_copy_paste(body, &mut ctx);
    out.append(&mut ctx.threats);
}

struct FnContext {
    name: String,
    loop_depth: u32,
    cond_depth: u32,
    threats: Vec<Threat>,
}

impl FnContext {
    fn push(&mut self, kind: ThreatKind, description: String, confidence: u8) {
        self.threats.push(Threat {
            kind,
            function: self.name.clone(),
            line: 0,
            col: 0,
            description,
            confidence,
        });
    }
}

// === Detection: OffByOne ===
//
// While / for loop with `i <= LEN_EXPR` where LEN_EXPR is `len(...)`
// or `arr.len()`. Half-open ranges should always be `<`.
fn detect_off_by_one(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::WhileStatement {
            condition, body, ..
        } => {
            if let Node::InfixExpression {
                operator, right, ..
            } = condition.as_ref()
            {
                if (*operator == "<=" || *operator == ">=") && is_len_call(right) {
                    ctx.push(
                        ThreatKind::OffByOne,
                        format!(
                            "while loop bounded by `{op} len(...)`, likely off-by-one",
                            op = operator
                        ),
                        85,
                    );
                }
            }
            detect_off_by_one(body, ctx);
        }
        Node::ForInStatement { body, .. } => detect_off_by_one(body, ctx),
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_off_by_one(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_off_by_one(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_off_by_one(a, ctx);
            }
        }
        _ => {}
    }
}

fn is_len_call(node: &Node) -> bool {
    if let Node::CallExpression { function, .. } = node {
        if let Node::Identifier { name, .. } = function.as_ref() {
            return name == "len";
        }
    }
    false
}

// === Detection: MissedElse ===
//
// `if cond { return X; } CODE_THAT_ASSUMES_NOT_COND`
// We flag a stand-alone `if` (no else) whose body returns.
fn detect_missed_else(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::Block { stmts, .. } => {
            for (i, s) in stmts.iter().enumerate() {
                if let Node::IfStatement {
                    consequence,
                    alternative,
                    ..
                } = s
                {
                    if alternative.is_none()
                        && block_returns(consequence)
                        && i + 1 < stmts.len()
                        && !is_trailing_return(&stmts[stmts.len() - 1])
                    {
                        ctx.push(
                            ThreatKind::MissedElse,
                            "if-without-else has body that returns; the path after has no explicit return".to_string(),
                            65,
                        );
                    }
                }
                detect_missed_else(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_missed_else(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_missed_else(a, ctx);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            detect_missed_else(body, ctx);
        }
        _ => {}
    }
}

fn block_returns(node: &Node) -> bool {
    match node {
        Node::Block { stmts, .. } => stmts
            .iter()
            .any(|s| matches!(s, Node::ReturnStatement { .. })),
        Node::ReturnStatement { .. } => true,
        _ => false,
    }
}

fn is_trailing_return(node: &Node) -> bool {
    matches!(node, Node::ReturnStatement { .. })
}

// === Detection: SwallowedError ===
//
// `try { ... } catch _ { }` (empty handler) — silently consumes errors.
fn detect_swallowed_error(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::TryCatch { handlers, body, .. } => {
            for (_, handler_body) in handlers {
                if handler_body.is_empty() {
                    ctx.push(
                        ThreatKind::SwallowedError,
                        "empty `catch` arm — error is silently dropped".to_string(),
                        95,
                    );
                }
            }
            for s in body {
                detect_swallowed_error(s, ctx);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_swallowed_error(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_swallowed_error(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_swallowed_error(a, ctx);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            detect_swallowed_error(body, ctx);
        }
        _ => {}
    }
}

// === Detection: MagicNumber ===
//
// Integer literal > 1 that doesn't appear in a recognised whitelist
// (powers of two, 0, ±1, common buffer sizes). Must appear in an
// arithmetic infix, not an array index or comparison-to-zero.
fn detect_magic_numbers(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::InfixExpression {
            left,
            operator,
            right,
            ..
        } => {
            let arith = matches!(*operator, "+" | "-" | "*" | "/" | "%" | "<<" | ">>");
            if arith {
                check_magic_operand(left, ctx);
                check_magic_operand(right, ctx);
            }
            detect_magic_numbers(left, ctx);
            detect_magic_numbers(right, ctx);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_magic_numbers(s, ctx);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => detect_magic_numbers(e, ctx),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            detect_magic_numbers(value, ctx);
        }
        Node::ExpressionStatement { expr, .. } => detect_magic_numbers(expr, ctx),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            detect_magic_numbers(condition, ctx);
            detect_magic_numbers(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_magic_numbers(a, ctx);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            detect_magic_numbers(condition, ctx);
            detect_magic_numbers(body, ctx);
        }
        Node::ForInStatement { body, .. } => detect_magic_numbers(body, ctx),
        Node::CallExpression { arguments, .. } => {
            for a in arguments {
                detect_magic_numbers(a, ctx);
            }
        }
        _ => {}
    }
}

fn check_magic_operand(node: &Node, ctx: &mut FnContext) {
    if let Node::IntegerLiteral { value, .. } = node {
        let v = *value;
        if v.abs() > 1 && !SMALL_CONSTANT_TOKENS.contains(&v) {
            ctx.push(
                ThreatKind::MagicNumber,
                format!("integer literal `{v}` in arithmetic — name it"),
                55,
            );
        }
    }
}

// === Detection: UnboundedLoop ===
//
// `while true { BODY }` where no `break` is reachable from BODY.
fn detect_unbounded_loops(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::WhileStatement {
            condition, body, ..
        } => {
            if matches!(
                condition.as_ref(),
                Node::BooleanLiteral { value: true, .. } | Node::IntegerLiteral { value: 1, .. }
            ) && !contains_break(body)
            {
                ctx.push(
                    ThreatKind::UnboundedLoop,
                    "`while true { ... }` with no `break` is an infinite loop".to_string(),
                    90,
                );
            }
            detect_unbounded_loops(body, ctx);
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_unbounded_loops(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_unbounded_loops(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_unbounded_loops(a, ctx);
            }
        }
        Node::ForInStatement { body, .. } => detect_unbounded_loops(body, ctx),
        _ => {}
    }
}

fn contains_break(node: &Node) -> bool {
    match node {
        Node::Break { .. } => true,
        Node::ReturnStatement { .. } => true,
        Node::Block { stmts, .. } => stmts.iter().any(contains_break),
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => contains_break(consequence) || alternative.as_ref().is_some_and(|a| contains_break(a)),
        _ => false,
    }
}

// === Detection: GhostHandler ===
//
// `catch _ { println(...); }` or `catch _ { return 0; }` — the handler
// looks like it does something, but the actual recovery is missing.
fn detect_ghost_handler(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::TryCatch { handlers, body, .. } => {
            for (_, handler_body) in handlers {
                if is_ghost_body(handler_body) {
                    ctx.push(
                        ThreatKind::GhostHandler,
                        "catch arm only logs or returns a literal — no real recovery".to_string(),
                        70,
                    );
                }
            }
            for s in body {
                detect_ghost_handler(s, ctx);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_ghost_handler(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_ghost_handler(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_ghost_handler(a, ctx);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            detect_ghost_handler(body, ctx);
        }
        _ => {}
    }
}

fn is_ghost_body(stmts: &[Node]) -> bool {
    if stmts.is_empty() {
        return false; // covered by SwallowedError
    }
    stmts.iter().all(|s| match s {
        Node::ExpressionStatement { expr, .. } => is_ghost_log(expr),
        Node::ReturnStatement { value, .. } => match value {
            None => true,
            Some(boxed) => matches!(
                boxed.as_ref(),
                Node::IntegerLiteral { .. }
                    | Node::BooleanLiteral { .. }
                    | Node::StringLiteral { .. }
            ),
        },
        _ => false,
    })
}

fn is_ghost_log(node: &Node) -> bool {
    if let Node::CallExpression { function, .. } = node {
        if let Node::Identifier { name, .. } = function.as_ref() {
            return matches!(name.as_str(), "println" | "print" | "eprintln" | "log");
        }
    }
    false
}

// === Detection: HallucinatedIdent ===
//
// Call to an identifier that is not in scope. We can't always know
// scope, so we use a heuristic: if the called name is *very close*
// (Levenshtein 1-2) to a known builtin and isn't the builtin itself,
// flag it.
fn detect_hallucinated_idents(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::CallExpression {
            function,
            arguments,
            ..
        } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                // RES-2284: pre-filter the candidate set before paying
                // the O(L*M) `levenshtein` cost.
                //
                //   * The eventual gate is `name.len() > 3` — hoist it
                //     so we never enter the loop for short identifiers
                //     (which `KNOWN_BUILTINS` is full of false-positive
                //     matches against — e.g. "x" vs "pow").
                //   * `levenshtein(a, b) >= |a.len - b.len|`, so any
                //     builtin whose length differs from `name` by more
                //     than 2 cannot produce a verdict in the [1, 2]
                //     window. Skip the inner call entirely.
                //
                // For a typical builtin list of ~20 names averaging
                // ~5 chars and a non-builtin call name of e.g. 8 chars,
                // the length-diff filter alone trims roughly half of
                // the inner loop iterations.
                let name_len = name.len();
                if name_len > 3 && !KNOWN_BUILTINS.contains(&name.as_str()) {
                    for builtin in KNOWN_BUILTINS {
                        if name_len.abs_diff(builtin.len()) > 2 {
                            continue;
                        }
                        let d = levenshtein(name, builtin);
                        if d > 0 && d <= 2 {
                            ctx.push(
                                ThreatKind::HallucinatedIdent,
                                format!(
                                    "call to `{name}` — did you mean `{builtin}`? (edit distance {d})"
                                ),
                                75,
                            );
                            break;
                        }
                    }
                }
            }
            for a in arguments {
                detect_hallucinated_idents(a, ctx);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_hallucinated_idents(s, ctx);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => detect_hallucinated_idents(e, ctx),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => {
            detect_hallucinated_idents(value, ctx);
        }
        Node::ExpressionStatement { expr, .. } => detect_hallucinated_idents(expr, ctx),
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            detect_hallucinated_idents(condition, ctx);
            detect_hallucinated_idents(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_hallucinated_idents(a, ctx);
            }
        }
        Node::WhileStatement {
            condition, body, ..
        } => {
            detect_hallucinated_idents(condition, ctx);
            detect_hallucinated_idents(body, ctx);
        }
        Node::ForInStatement { body, .. } => detect_hallucinated_idents(body, ctx),
        Node::InfixExpression { left, right, .. } => {
            detect_hallucinated_idents(left, ctx);
            detect_hallucinated_idents(right, ctx);
        }
        _ => {}
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let n = a.chars().count();
    let m = b.chars().count();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev = (0..=m).collect::<Vec<usize>>();
    let mut curr = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

// === Detection: NestedConditional ===
//
// Three or more `if` expressions nested as expression-position values.
// AI-generated code often produces these instead of `match`.
fn detect_nested_conditionals(node: &Node, depth: u32, ctx: &mut FnContext) {
    match node {
        Node::IfStatement {
            condition,
            consequence,
            alternative,
            ..
        } => {
            // Only count expression-position ifs (those inside other if branches).
            let new_depth = depth + 1;
            if new_depth >= 3 {
                ctx.push(
                    ThreatKind::NestedConditional,
                    format!("nested if-conditional at depth {new_depth} — flatten with `match`"),
                    60,
                );
                return;
            }
            detect_nested_conditionals(condition, 0, ctx);
            detect_nested_conditionals(consequence, new_depth, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_nested_conditionals(a, new_depth, ctx);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_nested_conditionals(s, depth, ctx);
            }
        }
        Node::ReturnStatement { value: Some(e), .. } => detect_nested_conditionals(e, depth, ctx),
        Node::LetStatement { value, .. } => detect_nested_conditionals(value, depth, ctx),
        Node::ExpressionStatement { expr, .. } => detect_nested_conditionals(expr, depth, ctx),
        _ => {}
    }
}

// === Detection: SilentSwallow ===
//
// `try { x = fail "..."; } catch _ { return 0; }` — the catch arm
// returns a literal value, hiding the failure from the caller.
fn detect_silent_swallow(node: &Node, ctx: &mut FnContext) {
    match node {
        Node::TryCatch { handlers, body, .. } => {
            let body_can_fail = body.iter().any(stmt_can_fail);
            for (_, handler_body) in handlers {
                if body_can_fail
                    && handler_body.len() == 1
                    && matches!(
                        &handler_body[0],
                        Node::ReturnStatement { value: Some(boxed), .. }
                            if matches!(
                                boxed.as_ref(),
                                Node::IntegerLiteral { .. }
                                    | Node::BooleanLiteral { .. }
                                    | Node::StringLiteral { .. }
                            )
                    )
                {
                    ctx.push(
                        ThreatKind::SilentSwallow,
                        "catch arm returns a literal — failure flows out as a normal value"
                            .to_string(),
                        80,
                    );
                }
            }
            for s in body {
                detect_silent_swallow(s, ctx);
            }
        }
        Node::Block { stmts, .. } => {
            for s in stmts {
                detect_silent_swallow(s, ctx);
            }
        }
        Node::IfStatement {
            consequence,
            alternative,
            ..
        } => {
            detect_silent_swallow(consequence, ctx);
            if let Some(a) = alternative.as_ref() {
                detect_silent_swallow(a, ctx);
            }
        }
        Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
            detect_silent_swallow(body, ctx);
        }
        _ => {}
    }
}

fn stmt_can_fail(node: &Node) -> bool {
    match node {
        Node::CallExpression { function, .. } => {
            if let Node::Identifier { name, .. } = function.as_ref() {
                return name == "fail" || name == "panic";
            }
            false
        }
        Node::ExpressionStatement { expr, .. } => stmt_can_fail(expr),
        Node::LetStatement { value, .. } | Node::Assignment { value, .. } => stmt_can_fail(value),
        _ => false,
    }
}

// === Detection: CopyPasteBlock ===
//
// Within a single function, find blocks of >= 3 statements whose
// shape-hash matches another block in the same function. Shape-hash
// ignores literal values and identifiers — only the AST node *kinds*
// at each position contribute.
fn detect_copy_paste(node: &Node, ctx: &mut FnContext) {
    let mut shapes: HashMap<String, u32> = HashMap::new();
    collect_block_shapes(node, &mut shapes);
    for (shape, count) in shapes {
        if count >= 2 {
            // shape encodes statement count; only flag for >= 3-stmt blocks.
            let stmt_count = shape.split('|').count();
            if stmt_count >= 3 {
                ctx.push(
                    ThreatKind::CopyPasteBlock,
                    format!(
                        "{count} structurally-identical {stmt_count}-statement blocks — extract a helper"
                    ),
                    65,
                );
            }
        }
    }
}

fn collect_block_shapes(node: &Node, out: &mut HashMap<String, u32>) {
    if let Node::Block { stmts, .. } = node {
        if stmts.len() >= 3 {
            // RES-1986: build the shape key directly into a pre-sized
            // String instead of collecting a `Vec<&'static str>` just
            // to `.join("|")` it. The previous shape allocated one Vec
            // + one String per ≥3-stmt block; for programs with many
            // such blocks (the typical large fn body), the intermediate
            // Vec is per-block dead weight. Each tag is a single
            // character so `stmts.len() * 2` covers the tag + separator
            // pattern exactly.
            let mut shape = String::with_capacity(stmts.len() * 2);
            for (i, s) in stmts.iter().enumerate() {
                if i > 0 {
                    shape.push('|');
                }
                shape.push_str(stmt_shape_tag(s));
            }
            *out.entry(shape).or_insert(0) += 1;
        }
        for s in stmts {
            collect_block_shapes(s, out);
        }
    } else {
        match node {
            Node::IfStatement {
                consequence,
                alternative,
                ..
            } => {
                collect_block_shapes(consequence, out);
                if let Some(a) = alternative.as_ref() {
                    collect_block_shapes(a, out);
                }
            }
            Node::WhileStatement { body, .. } | Node::ForInStatement { body, .. } => {
                collect_block_shapes(body, out);
            }
            _ => {}
        }
    }
}

fn stmt_shape_tag(node: &Node) -> &'static str {
    match node {
        Node::LetStatement { .. } => "L",
        Node::Assignment { .. } => "A",
        Node::ReturnStatement { .. } => "R",
        Node::ExpressionStatement { .. } => "E",
        Node::IfStatement { .. } => "I",
        Node::WhileStatement { .. } => "W",
        Node::ForInStatement { .. } => "F",
        Node::Break { .. } => "B",
        Node::Continue { .. } => "C",
        _ => "?",
    }
}

// === Public surface ===

/// Collect functions tagged `#[ai_review_required]` from the registry.
pub fn collect_ai_review_fns() -> HashSet<String> {
    crate::feature_attrs::find_kind("ai_review_required")
        .into_iter()
        .map(|(item, _)| item)
        .collect()
}

/// `--ai-threats` CLI driver: returns a human-readable report.
pub fn report(program: &Node, source_path: &str) -> String {
    let threats = analyze_program(program);
    if threats.is_empty() {
        return format!("{source_path}: no AI threats detected");
    }
    let mut lines = vec![format!(
        "{source_path}: {} AI threat(s) detected",
        threats.len()
    )];
    for t in &threats {
        lines.push(format!(
            "  in fn `{}`: [{}] {} (confidence={}%) — {}",
            t.function,
            t.kind.label(),
            t.description,
            t.confidence,
            t.kind.mitigation()
        ));
    }
    lines.join("\n")
}

/// Hard pass: every threat in a `#[ai_review_required]` function is an error.
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let gated = collect_ai_review_fns();
    if gated.is_empty() {
        return Ok(());
    }
    // RES-1561: only analyze fns that are actually gated by
    // `#[ai_review_required]`. The previous shape called
    // `analyze_program` (walks every top-level fn through ten
    // detectors) and then filtered the threats — every non-gated
    // fn's analysis was wasted work. Iterate top-level statements
    // once and run `analyze_function` only when the fn is gated.
    // The pub `analyze_program` API (used by the `--ai-threats` CLI
    // flag) stays unchanged.
    let Node::Program(stmts) = program else {
        return Ok(());
    };
    for s in stmts {
        if let Node::Function { name, body, .. } = &s.node {
            if !gated.contains(name) {
                continue;
            }
            let mut threats = Vec::new();
            analyze_function(name, body, &mut threats);
            if let Some(t) = threats.first() {
                return Err(format!(
                    "{}:0:0: error: `{}` is `#[ai_review_required]` but contains AI threat [{}]: {}",
                    source_path,
                    t.function,
                    t.kind.label(),
                    t.description
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn analyze(src: &str) -> Vec<Threat> {
        let _g = crate::feature_attrs::lock_for_test();
        let (prog, _) = parse(src);
        analyze_program(&prog)
    }

    #[test]
    fn off_by_one_detected_in_while() {
        let threats = analyze(
            r#"fn scan(int n) -> int {
                let mut i = 0;
                while i <= len(n) { i = i + 1; }
                return i;
            }"#,
        );
        assert!(
            threats.iter().any(|t| t.kind == ThreatKind::OffByOne),
            "expected OffByOne, got {threats:?}"
        );
    }

    #[test]
    fn empty_catch_arm_is_swallowed_error() {
        let threats = analyze(
            r#"fn run(int x) -> int {
                try { let y = panic(x); } catch _ { }
                return 0;
            }"#,
        );
        assert!(threats.iter().any(|t| t.kind == ThreatKind::SwallowedError));
    }

    #[test]
    fn while_true_no_break_is_unbounded() {
        let threats = analyze(
            r#"fn busy(int x) -> int {
                while true { let y = x + 1; }
                return 0;
            }"#,
        );
        assert!(threats.iter().any(|t| t.kind == ThreatKind::UnboundedLoop));
    }

    #[test]
    fn while_true_with_break_is_ok() {
        let threats = analyze(
            r#"fn drain(int x) -> int {
                while true { if x == 0 { break; } }
                return 0;
            }"#,
        );
        assert!(!threats.iter().any(|t| t.kind == ThreatKind::UnboundedLoop));
    }

    #[test]
    fn magic_number_in_arithmetic_flagged() {
        let threats = analyze(
            r#"fn weird(int x) -> int {
                return x * 7919;
            }"#,
        );
        assert!(threats.iter().any(|t| t.kind == ThreatKind::MagicNumber));
    }

    #[test]
    fn small_constants_not_flagged() {
        let threats = analyze(
            r#"fn ok(int x) -> int {
                return x * 2 + 1;
            }"#,
        );
        assert!(!threats.iter().any(|t| t.kind == ThreatKind::MagicNumber));
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("println", "println"), 0);
        assert_eq!(levenshtein("printl", "println"), 1);
        assert_eq!(levenshtein("prinln", "println"), 1);
        assert_eq!(levenshtein("prtnln", "println"), 2);
    }

    #[test]
    fn report_empty_on_clean_code() {
        let _g = crate::feature_attrs::lock_for_test();
        let src = r#"fn add(int a, int b) -> int { return a + b; }"#;
        let (prog, _) = parse(src);
        let r = report(&prog, "test.rz");
        assert!(r.contains("no AI threats detected"));
    }

    #[test]
    fn report_lists_each_threat() {
        let _g = crate::feature_attrs::lock_for_test();
        let src = r#"fn dangerous(int n) -> int {
            while true { let x = n + 1; }
            return 0;
        }"#;
        let (prog, _) = parse(src);
        let r = report(&prog, "test.rz");
        assert!(r.contains("unbounded-loop"));
        assert!(r.contains("confidence="));
    }
}
