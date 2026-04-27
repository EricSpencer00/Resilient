//! RES-343: conditional compilation via `#[cfg(...)]` attributes.
//!
//! Embedded programs need to compile different code for different targets
//! (e.g. `printf` on hosted, RTT on bare-metal). The `#[cfg(...)]` attribute
//! gates a top-level declaration on a feature flag or target triple. Items
//! whose predicate is false are stripped from the AST **before** type
//! checking — they are not lowered, not type-checked, and not executed,
//! which matches the Rust mental model and the ticket's "stripped before
//! type checking" requirement.
//!
//! ## Surface syntax
//!
//! ```text
//! #[cfg(feature = "std")]
//! fn log(string msg) { println(msg); }
//!
//! #[cfg(not(feature = "std"))]
//! fn log(string msg) { rtt_write(msg); }
//!
//! #[cfg(target = "thumbv7em-none-eabihf")]
//! fn boot() { ... }
//! ```
//!
//! Active features are passed via repeatable `--feature NAME` CLI flags.
//! The active target triple is passed via `--target TRIPLE`. Both default
//! to empty.
//!
//! ## Feature isolation
//!
//! All cfg logic lives in this module. The core files only touch:
//!
//! * `lexer_logos.rs` `<EXTENSION_TOKENS>` — the `#[` opener.
//! * `main.rs` `<EXTENSION_TOKENS>` — same opener for the hand-rolled lexer.
//! * `main.rs` `parse_statement` — one dispatch arm that calls
//!   [`parse_cfg_attribute`] when the lexer emits `Token::HashLeftBracket`.
//!
//! No typechecker, no runtime, no VM hooks. Stripping is a pure parser-time
//! transform: a disabled item is dropped on the floor and never appears in
//! the AST, so downstream passes see exactly what they would see if the
//! user hadn't written the gated code at all.

use std::collections::HashSet;
use std::sync::RwLock;

/// RES-343: process-wide active-cfg state. The CLI driver populates this
/// once, before parsing begins. Tests reset it via [`reset_for_test`].
///
/// The shape mirrors how `bounds_check::set_deny_unproven_bounds` threads
/// CLI flags into a parser-time pass without widening the `Parser`
/// constructor — this is the established pattern in CLAUDE.md and keeps
/// the `main.rs` extension footprint minimal (one CLI-arg arm + one
/// dispatch line).
#[derive(Debug, Default, Clone)]
pub struct CfgConfig {
    /// `--feature NAME` (repeatable). A `#[cfg(feature = "X")]` attribute
    /// is satisfied iff `"X"` is in this set.
    pub features: HashSet<String>,
    /// `--target TRIPLE`. A `#[cfg(target = "T")]` attribute is satisfied
    /// iff `target.as_deref() == Some("T")`.
    pub target: Option<String>,
}

impl CfgConfig {
    /// Construct a config with the given active features. Convenience for
    /// tests and CLI wiring.
    #[allow(dead_code)] // used by tests; CLI builds the struct literal directly
    pub fn new<I>(features: I, target: Option<String>) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        Self {
            features: features.into_iter().collect(),
            target,
        }
    }
}

static ACTIVE_CFG: RwLock<Option<CfgConfig>> = RwLock::new(None);

/// Install the active cfg config for the rest of this process. Called by
/// the driver in `main.rs` after CLI parsing. Subsequent calls overwrite —
/// the LSP / REPL re-installs on each compile.
pub fn set_active_config(cfg: CfgConfig) {
    if let Ok(mut guard) = ACTIVE_CFG.write() {
        *guard = Some(cfg);
    }
}

/// Read the currently installed config. Returns the default (no features,
/// no target) when nothing has been installed — that way unit tests that
/// drive the parser directly don't need to set anything up.
pub fn active_config() -> CfgConfig {
    ACTIVE_CFG
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

/// Reset the active config. Used by `#[cfg(test)]` to keep tests
/// independent in the face of the process-wide `RwLock`.
#[cfg(test)]
pub fn reset_for_test() {
    if let Ok(mut guard) = ACTIVE_CFG.write() {
        *guard = None;
    }
}

/// RES-343: a parsed cfg predicate. Surface syntax kept narrow — only the
/// shapes the ticket lists as acceptance criteria. Logical combinators
/// (`any`, `all`) are tracked as a follow-up; `not(...)` is in scope here
/// because the ticket explicitly calls it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgPredicate {
    /// `feature = "name"` — true iff `name` is in `CfgConfig::features`.
    Feature(String),
    /// `target = "triple"` — true iff `triple` matches `CfgConfig::target`.
    Target(String),
    /// `not(<inner>)` — logical negation.
    Not(Box<CfgPredicate>),
    /// Parse error placeholder. Treated as `false` so a malformed cfg
    /// drops the gated item; the parse error itself surfaces via the
    /// normal diagnostic channel.
    Invalid,
}

impl CfgPredicate {
    /// Evaluate the predicate against the active config. A malformed
    /// predicate returns `false` so a bad cfg never accidentally
    /// includes code the user clearly intended to gate.
    pub fn eval(&self, cfg: &CfgConfig) -> bool {
        match self {
            CfgPredicate::Feature(name) => cfg.features.contains(name),
            CfgPredicate::Target(triple) => cfg.target.as_deref() == Some(triple.as_str()),
            CfgPredicate::Not(inner) => !inner.eval(cfg),
            CfgPredicate::Invalid => false,
        }
    }
}

/// Result of parsing one `#[cfg(...)]` attribute. The parser hook in
/// `main.rs` consults `is_active` to decide whether to keep or drop the
/// following item.
#[derive(Debug, Clone)]
pub struct CfgAttribute {
    /// The parsed predicate. Retained on the struct so future tooling
    /// (LSP, lints, audit reports) can inspect the original cfg
    /// expression after evaluation; not currently consumed inside the
    /// compiler driver.
    #[allow(dead_code)]
    pub predicate: CfgPredicate,
    pub is_active: bool,
}

impl CfgAttribute {
    /// Build from a predicate, evaluating against the currently installed
    /// config. Centralised so every call site uses the same evaluator.
    pub fn from_predicate(predicate: CfgPredicate) -> Self {
        let is_active = predicate.eval(&active_config());
        CfgAttribute {
            predicate,
            is_active,
        }
    }
}

// ---------------------------------------------------------------------------
// Parser entry point.
//
// The body of the cfg attribute lives here so `main.rs` only adds a single
// dispatch arm. The parser hook signature mirrors the existing
// `parse_attributed_item` / `parse_repr_attribute` shape: it receives `&mut
// crate::Parser`, mutates the cursor, and returns the next AST node — or
// `None` if the gated item was stripped.
//
// Entry condition: `parser.current_token == Token::HashLeftBracket`.
// Exit condition: cursor is positioned at the token after the gated item
// (or after the consumed item-skipping recovery, on a stripped branch).

use crate::{Parser, Token};

/// RES-343: parse `#[cfg(<predicate>)]` followed by the gated item. Returns
/// `Some(node)` if the predicate is active (the item is kept), `None` if
/// the item is stripped from the AST. The caller is `Parser::parse_statement`
/// in `main.rs`; the dispatch hook lives in the existing match arm for the
/// new `Token::HashLeftBracket`.
///
/// Error-recovery strategy: if the attribute itself is malformed, record an
/// error, treat the predicate as `Invalid` (→ false), and still consume one
/// following item so the parser stays aligned. This matches the
/// `parse_attributed_item` recovery behaviour for `@pure` typos.
pub fn parse_cfg_attribute(parser: &mut Parser) -> Option<crate::Node> {
    debug_assert_eq!(parser.current_token, Token::HashLeftBracket);
    parser.next_token(); // consume `#[`

    // Expect identifier `cfg`. Anything else is a parse error; recover by
    // skipping to `]` and parsing the next item normally (no gating).
    let attr_name = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "expected attribute name after `#[`, found {} (only `cfg` is supported)",
                tok
            ));
            skip_until_close_bracket(parser);
            return parser.parse_statement();
        }
    };
    if attr_name != "cfg" {
        parser.record_error(format!(
            "unknown attribute `#[{}]`. Known: `#[cfg(...)]`",
            attr_name
        ));
        skip_until_close_bracket(parser);
        return parser.parse_statement();
    }
    parser.next_token(); // skip `cfg`

    // `(` opening the predicate.
    if !matches!(parser.current_token, Token::LeftParen) {
        let tok = parser.current_token.clone();
        parser.record_error(format!("expected `(` after `#[cfg`, found {}", tok));
        skip_until_close_bracket(parser);
        return parser.parse_statement();
    }
    parser.next_token(); // skip `(`

    let predicate = parse_predicate(parser);

    if !matches!(parser.current_token, Token::RightParen) {
        let tok = parser.current_token.clone();
        parser.record_error(format!("expected `)` to close `#[cfg(...)`, found {}", tok));
        skip_until_close_bracket(parser);
        return parser.parse_statement();
    }
    parser.next_token(); // skip `)`

    if !matches!(parser.current_token, Token::RightBracket) {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "expected `]` to close `#[cfg(...)]`, found {}",
            tok
        ));
        skip_until_close_bracket(parser);
        return parser.parse_statement();
    }
    parser.next_token(); // skip `]`

    let attr = CfgAttribute::from_predicate(predicate);

    if attr.is_active {
        // Active: parse the next item normally.
        parser.parse_statement()
    } else {
        // Inactive: parse the next item but throw it away. We still parse
        // (rather than scanning tokens) because that is the safest way to
        // keep the cursor aligned — a half-skipped fn body will cascade
        // unrelated errors. The cost is small: typechecking and lowering
        // happen later.
        let _ = parser.parse_statement();
        None
    }
}

/// Recursive predicate parser. Handles `feature = "x"`, `target = "x"`,
/// and `not(<inner>)`. On a malformed predicate, records an error and
/// returns `CfgPredicate::Invalid`; the caller continues to advance the
/// cursor through the surrounding `)` / `]`.
fn parse_predicate(parser: &mut Parser) -> CfgPredicate {
    let kind = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "expected `feature`, `target`, or `not` inside `#[cfg(...)]`, found {}",
                tok
            ));
            return CfgPredicate::Invalid;
        }
    };
    parser.next_token();

    match kind.as_str() {
        "feature" => parse_kv_predicate(parser, "feature", CfgPredicate::Feature),
        "target" => parse_kv_predicate(parser, "target", CfgPredicate::Target),
        "not" => parse_not_predicate(parser),
        other => {
            parser.record_error(format!(
                "unknown cfg predicate `{}`. Supported: `feature`, `target`, `not`",
                other
            ));
            CfgPredicate::Invalid
        }
    }
}

fn parse_kv_predicate(
    parser: &mut Parser,
    key: &str,
    ctor: fn(String) -> CfgPredicate,
) -> CfgPredicate {
    if !matches!(parser.current_token, Token::Assign) {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "expected `=` after `{}` in `#[cfg]`, found {}",
            key, tok
        ));
        return CfgPredicate::Invalid;
    }
    parser.next_token(); // skip `=`

    let value = match &parser.current_token {
        Token::StringLiteral(s) => s.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!(
                "expected string literal after `{} =` in `#[cfg]`, found {}",
                key, tok
            ));
            return CfgPredicate::Invalid;
        }
    };
    parser.next_token(); // skip the string literal
    ctor(value)
}

fn parse_not_predicate(parser: &mut Parser) -> CfgPredicate {
    if !matches!(parser.current_token, Token::LeftParen) {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "expected `(` after `not` in `#[cfg]`, found {}",
            tok
        ));
        return CfgPredicate::Invalid;
    }
    parser.next_token(); // skip `(`

    let inner = parse_predicate(parser);

    if !matches!(parser.current_token, Token::RightParen) {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "expected `)` after `not(...)` in `#[cfg]`, found {}",
            tok
        ));
        return CfgPredicate::Invalid;
    }
    parser.next_token(); // skip `)`

    CfgPredicate::Not(Box::new(inner))
}

/// Recovery helper: scan forward until we land just past the next `]`,
/// or hit EOF. Used by every error-path branch above so the parser cursor
/// is always positioned at a clean statement boundary before the next
/// `parse_statement` call.
fn skip_until_close_bracket(parser: &mut Parser) {
    while !matches!(parser.current_token, Token::RightBracket | Token::Eof) {
        parser.next_token();
    }
    if matches!(parser.current_token, Token::RightBracket) {
        parser.next_token();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Node;
    use std::sync::Mutex;

    /// Tests in this module mutate the process-wide `ACTIVE_CFG`. Serialise
    /// them with a Mutex so concurrent runs don't see each other's state.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn parse_with_cfg(src: &str, cfg: CfgConfig) -> (Node, Vec<String>) {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        set_active_config(cfg);
        let result = crate::parse(src);
        reset_for_test();
        result
    }

    fn top_level_fn_names(program: &Node) -> Vec<String> {
        let stmts = match program {
            Node::Program(s) => s,
            _ => panic!("expected Program"),
        };
        stmts
            .iter()
            .filter_map(|s| match &s.node {
                Node::Function { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    fn top_level_struct_names(program: &Node) -> Vec<String> {
        let stmts = match program {
            Node::Program(s) => s,
            _ => panic!("expected Program"),
        };
        stmts
            .iter()
            .filter_map(|s| match &s.node {
                Node::StructDecl { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn predicate_eval_feature() {
        let cfg = CfgConfig::new(["std".to_string()], None);
        assert!(CfgPredicate::Feature("std".into()).eval(&cfg));
        assert!(!CfgPredicate::Feature("alloc".into()).eval(&cfg));
    }

    #[test]
    fn predicate_eval_target() {
        let cfg = CfgConfig::new(std::iter::empty(), Some("thumbv7em-none-eabihf".into()));
        assert!(CfgPredicate::Target("thumbv7em-none-eabihf".into()).eval(&cfg));
        assert!(!CfgPredicate::Target("x86_64-unknown-linux-gnu".into()).eval(&cfg));
    }

    #[test]
    fn predicate_eval_not() {
        let cfg = CfgConfig::new(["std".to_string()], None);
        let p = CfgPredicate::Not(Box::new(CfgPredicate::Feature("std".into())));
        assert!(!p.eval(&cfg));
        let p = CfgPredicate::Not(Box::new(CfgPredicate::Feature("nope".into())));
        assert!(p.eval(&cfg));
    }

    #[test]
    fn invalid_predicate_evals_false() {
        let cfg = CfgConfig::new(["std".to_string()], None);
        assert!(!CfgPredicate::Invalid.eval(&cfg));
    }

    #[test]
    fn cfg_feature_enabled_keeps_item() {
        let src = r#"
            #[cfg(feature = "std")]
            fn log(int x) { return x; }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::new(["std".to_string()], None);
        let (program, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        let names = top_level_fn_names(&program);
        assert!(names.contains(&"log".to_string()), "got {:?}", names);
        assert!(names.contains(&"main".to_string()), "got {:?}", names);
    }

    #[test]
    fn cfg_feature_disabled_strips_item() {
        let src = r#"
            #[cfg(feature = "std")]
            fn log(int x) { return x; }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::default(); // no features
        let (program, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        let names = top_level_fn_names(&program);
        assert!(
            !names.contains(&"log".to_string()),
            "stripped fn leaked: {:?}",
            names
        );
        assert!(names.contains(&"main".to_string()), "got {:?}", names);
    }

    #[test]
    fn cfg_not_feature_inverts() {
        // `not(feature = "std")` keeps the item only when "std" is INACTIVE.
        let src = r#"
            #[cfg(not(feature = "std"))]
            fn log(int x) { return x; }
            fn main(int dummy) { return 0; }
        "#;

        // std absent → kept.
        let (prog_a, _) = parse_with_cfg(src, CfgConfig::default());
        assert!(top_level_fn_names(&prog_a).contains(&"log".to_string()));

        // std present → stripped.
        let cfg = CfgConfig::new(["std".to_string()], None);
        let (prog_b, _) = parse_with_cfg(src, cfg);
        assert!(!top_level_fn_names(&prog_b).contains(&"log".to_string()));
    }

    #[test]
    fn cfg_target_matches_triple() {
        let src = r#"
            #[cfg(target = "thumbv7em-none-eabihf")]
            fn boot(int dummy) { return 0; }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::new(
            std::iter::empty(),
            Some("thumbv7em-none-eabihf".to_string()),
        );
        let (prog, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        assert!(top_level_fn_names(&prog).contains(&"boot".to_string()));
    }

    #[test]
    fn cfg_target_mismatch_strips() {
        let src = r#"
            #[cfg(target = "thumbv7em-none-eabihf")]
            fn boot(int dummy) { return 0; }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::new(
            std::iter::empty(),
            Some("x86_64-unknown-linux-gnu".to_string()),
        );
        let (prog, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        assert!(!top_level_fn_names(&prog).contains(&"boot".to_string()));
    }

    #[test]
    fn cfg_gates_struct_decl() {
        // The ticket calls out functions AND structs. Make sure the same
        // pre-typecheck strip logic applies to a `struct` declaration.
        let src = r#"
            #[cfg(feature = "telemetry")]
            struct Reading { int value, int timestamp }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::default();
        let (prog, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        assert!(top_level_struct_names(&prog).is_empty());

        let cfg = CfgConfig::new(["telemetry".to_string()], None);
        let (prog, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        assert_eq!(top_level_struct_names(&prog), vec!["Reading".to_string()]);
    }

    #[test]
    fn alternate_implementations_select_by_feature() {
        // The "two alternative implementations" acceptance criterion.
        let src = r#"
            #[cfg(feature = "std")]
            fn impl_select(int x) { return x + 1; }
            #[cfg(not(feature = "std"))]
            fn impl_select(int x) { return x + 100; }
            fn main(int dummy) { return 0; }
        "#;

        // With "std": only the first body survives.
        let cfg = CfgConfig::new(["std".to_string()], None);
        let (prog, errs) = parse_with_cfg(src, cfg);
        assert!(errs.is_empty(), "unexpected parse errors: {:?}", errs);
        let stmts = match &prog {
            Node::Program(s) => s,
            _ => unreachable!(),
        };
        let select_count = stmts
            .iter()
            .filter(|s| matches!(&s.node, Node::Function { name, .. } if name == "impl_select"))
            .count();
        assert_eq!(
            select_count, 1,
            "expected exactly one impl_select fn after stripping"
        );

        // Without "std": only the bare-metal body survives.
        let (prog2, errs2) = parse_with_cfg(src, CfgConfig::default());
        assert!(errs2.is_empty(), "unexpected parse errors: {:?}", errs2);
        let stmts2 = match &prog2 {
            Node::Program(s) => s,
            _ => unreachable!(),
        };
        let select_count2 = stmts2
            .iter()
            .filter(|s| matches!(&s.node, Node::Function { name, .. } if name == "impl_select"))
            .count();
        assert_eq!(select_count2, 1);
    }

    #[test]
    fn unknown_attribute_records_error_and_recovers() {
        // `#[bogus]` is not a known attribute. The parser should record
        // an error but still produce an AST so downstream errors don't
        // cascade. The following `fn main` should still parse.
        let src = r#"
            #[bogus]
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::default();
        let (program, errs) = parse_with_cfg(src, cfg);
        assert!(!errs.is_empty(), "expected an error for unknown attribute");
        // `main` should still appear in the AST after recovery.
        assert!(top_level_fn_names(&program).contains(&"main".to_string()));
    }

    #[test]
    fn malformed_cfg_predicate_strips_item() {
        // `#[cfg(feature)]` is missing the `= "..."` payload. Treat as
        // Invalid (= false) so the gated item is stripped and surface a
        // diagnostic.
        let src = r#"
            #[cfg(feature)]
            fn dropped(int x) { return x; }
            fn main(int dummy) { return 0; }
        "#;
        let cfg = CfgConfig::new(["std".to_string()], None);
        let (program, errs) = parse_with_cfg(src, cfg);
        assert!(!errs.is_empty(), "expected a parse error for malformed cfg");
        assert!(!top_level_fn_names(&program).contains(&"dropped".to_string()));
    }
}
