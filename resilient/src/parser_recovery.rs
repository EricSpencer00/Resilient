//! RES-307: parser error recovery helpers.
//!
//! The parser used to abort on the first error path it could not
//! confidently advance past, leaving downstream diagnostics hidden
//! behind whichever syntax mistake the user happened to fix first.
//! Recovery flips the model: each error is recorded, the parser
//! resynchronises at the next plausible statement boundary, and
//! parsing continues so a single run reports every distinct
//! syntactic mistake.
//!
//! Two predicates classify tokens for the synchronization scanner
//! used by `Parser::synchronize_top_level` and
//! `Parser::synchronize_in_block` in `main.rs`:
//!
//! * [`starts_top_level_item`] — tokens that unambiguously start a
//!   fresh top-level construct (`fn`, `let`, `struct`, …). When the
//!   parser sees one of these after an error it can stop scanning
//!   and let `parse_statement` take over.
//! * [`starts_block_statement`] — tokens that start a fresh
//!   statement *inside* a `{ … }` block. A superset of the
//!   top-level predicate (every top-level form may also appear as a
//!   nested statement in the current grammar) plus a couple of
//!   block-only forms (e.g. `invariant`).
//!
//! Tokens are matched by reference so the predicates can be called
//! without cloning the lexer's current token. The helpers are
//! intentionally pure so they can be unit-tested without spinning
//! up a full `Parser`.
//!
//! `MAX_PARSE_ERRORS` caps how many distinct diagnostics one run
//! emits. The cap exists to keep pathological input (e.g. fuzzer
//! garbage that produces an error on every token) from blowing up
//! memory; in practice real programs produce a handful of errors
//! before the user fixes them.

use crate::Token;

/// Hard cap on the number of recorded parser errors per run.
///
/// Once this many diagnostics have been collected `Parser::record_error`
/// stops appending to the vector. The parser still drives forward to
/// EOF so the AST passed to later phases is shaped consistently — the
/// cap purely bounds diagnostic memory.
pub(crate) const MAX_PARSE_ERRORS: usize = 100;

/// Returns true if `tok` starts a top-level program item.
///
/// Used by the synchronization scanner in `parse_program`: after a
/// recorded error, the scanner advances tokens until it sees one of
/// these, the end of the input, or a `;` that would otherwise close
/// the current statement.
pub(crate) fn starts_top_level_item(tok: &Token) -> bool {
    matches!(
        tok,
        Token::Function
            | Token::Let
            | Token::Static
            | Token::Const
            | Token::Struct
            | Token::Impl
            | Token::Type
            | Token::Region
            | Token::Actor
            | Token::Extern
            | Token::Use
            | Token::If
            | Token::While
            | Token::For
            | Token::Return
            | Token::Assert
            | Token::Assume
            | Token::Live
            | Token::Try
            | Token::At
    )
}

/// Returns true if `tok` starts a statement legal at block scope.
///
/// A superset of [`starts_top_level_item`] — block scope additionally
/// admits the `invariant EXPR;` form (RES-222).
pub(crate) fn starts_block_statement(tok: &Token) -> bool {
    if starts_top_level_item(tok) {
        return true;
    }
    matches!(tok, Token::Invariant)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_starters_are_recognised() {
        // A handful of representative starters; the predicate is a
        // simple `matches!`, so listing every variant would just
        // mirror the predicate body.
        assert!(starts_top_level_item(&Token::Function));
        assert!(starts_top_level_item(&Token::Let));
        assert!(starts_top_level_item(&Token::Struct));
        assert!(starts_top_level_item(&Token::Return));
        assert!(starts_top_level_item(&Token::At));
    }

    #[test]
    fn non_starters_are_rejected_at_top_level() {
        assert!(!starts_top_level_item(&Token::Eof));
        assert!(!starts_top_level_item(&Token::Semicolon));
        assert!(!starts_top_level_item(&Token::RightBrace));
        assert!(!starts_top_level_item(&Token::Plus));
        // `invariant` is a block-only starter — must NOT be a
        // top-level starter or top-level recovery would try to begin
        // a statement on it and immediately error again.
        assert!(!starts_top_level_item(&Token::Invariant));
    }

    #[test]
    fn block_scope_includes_invariant() {
        assert!(starts_block_statement(&Token::Invariant));
        // And every top-level starter remains valid in block scope.
        assert!(starts_block_statement(&Token::Let));
        assert!(starts_block_statement(&Token::If));
    }
}
