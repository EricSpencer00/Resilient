//! RES-069 (G6 partial): Source-position infrastructure.
//!
//! `Pos` is a single point in the source file (1-indexed line and
//! column, plus a 0-indexed byte/char offset for fast slicing).
//! `Span` is a half-open `[start, end)` range over the source.
//! `Spanned<T>` wraps any value with a span — the next phase of G6 will
//! migrate AST nodes to `Spanned<Node>` so diagnostics from the
//! typechecker, interpreter, and verifier can all attribute back to the
//! exact source range.
//!
//! Today this module is the FOUNDATION ONLY: types + a lexer helper
//! method (see `Lexer::next_token_with_span` in main.rs) that emits a
//! `(Token, Span)` pair. AST migration is a follow-up under the same
//! ticket — until then, several constructors below are intentionally
//! unused outside the test suite.
#![allow(dead_code)]

use std::fmt;

/// A single source position. Line and column are 1-indexed for human
/// display; offset is the 0-indexed character index into the input
/// string and exists so we can slice the original source cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl Pos {
    pub const fn new(line: usize, column: usize, offset: usize) -> Self {
        Self { line, column, offset }
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Half-open `[start, end)` range. `end` points one past the last
/// character of the spanned region, so `end.offset - start.offset` is
/// the length in chars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: Pos,
    pub end: Pos,
}

impl Span {
    pub const fn new(start: Pos, end: Pos) -> Self {
        Self { start, end }
    }

    /// A zero-width span at a single point — useful for synthetic nodes
    /// that don't correspond to anything in the source (e.g. an
    /// implicit `return ();` injected at end-of-block).
    pub const fn point(pos: Pos) -> Self {
        Self { start: pos, end: pos }
    }

    /// Span covering both `self` and `other`. The result starts at the
    /// earlier of the two starts and ends at the later of the two ends.
    pub fn union(self, other: Span) -> Span {
        let start = if self.start.offset <= other.start.offset { self.start } else { other.start };
        let end = if self.end.offset >= other.end.offset { self.end } else { other.end };
        Span { start, end }
    }

    /// Length in chars.
    pub const fn len(&self) -> usize {
        self.end.offset.saturating_sub(self.start.offset)
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start.line == self.end.line {
            write!(f, "{}:{}-{}", self.start.line, self.start.column, self.end.column)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

/// Pairs any value with the source span it came from. The intended use
/// is `Spanned<Node>` once the AST migration lands.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub const fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Spanned<U> {
        Spanned { node: f(self.node), span: self.span }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned { node: &self.node, span: self.span }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_display_is_line_colon_column() {
        let p = Pos::new(7, 12, 99);
        assert_eq!(p.to_string(), "7:12");
    }

    #[test]
    fn span_union_covers_both_endpoints() {
        let a = Span::new(Pos::new(1, 1, 0), Pos::new(1, 4, 3));
        let b = Span::new(Pos::new(1, 7, 6), Pos::new(1, 10, 9));
        let u = a.union(b);
        assert_eq!(u.start, a.start);
        assert_eq!(u.end, b.end);
        assert_eq!(u.len(), 9);
    }

    #[test]
    fn span_union_is_commutative() {
        let a = Span::new(Pos::new(1, 1, 0), Pos::new(1, 4, 3));
        let b = Span::new(Pos::new(2, 1, 10), Pos::new(2, 5, 14));
        assert_eq!(a.union(b), b.union(a));
    }

    #[test]
    fn point_span_has_zero_length() {
        let s = Span::point(Pos::new(3, 5, 20));
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn span_display_is_compact_when_single_line() {
        let s = Span::new(Pos::new(2, 3, 10), Pos::new(2, 8, 15));
        assert_eq!(s.to_string(), "2:3-8");
    }

    #[test]
    fn span_display_uses_full_form_across_lines() {
        let s = Span::new(Pos::new(2, 3, 10), Pos::new(4, 1, 25));
        assert_eq!(s.to_string(), "2:3-4:1");
    }

    #[test]
    fn spanned_map_preserves_span() {
        let s = Spanned::new(7i32, Span::new(Pos::new(1, 1, 0), Pos::new(1, 2, 1)));
        let mapped = s.clone().map(|n| n.to_string());
        assert_eq!(mapped.node, "7");
        assert_eq!(mapped.span, s.span);
    }
}
