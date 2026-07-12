//! RES-069 / RES-115: source-position infrastructure for the
//! Resilient compiler pipeline, extracted into its own crate.
//!
//! `Pos` is a single point in the source file (1-indexed line and
//! column, plus a 0-indexed char offset for fast slicing). `Span`
//! is a half-open `[start, end)` range. `Spanned<T>` wraps any
//! value with a span — the intended use is `Spanned<Node>` on AST
//! nodes so diagnostics from the typechecker, interpreter, and
//! verifier can attribute back to the exact source range.
//!
//! `pos_from_byte` + the companion `build_line_table` helper are
//! the logos-lexer-side utility for translating byte offsets
//! emitted by `logos` into the 1-indexed line:col `Pos` shape the
//! rest of the pipeline expects (RES-110).
//!
//! This crate is std-only (spans are a compile-time concept) and
//! deliberately not depended on by `resilient-runtime` — that
//! sibling crate stays `no_std`-clean.
#![allow(dead_code)]

use std::fmt;

/// A single source position. Line and column are 1-indexed for human
/// display; offset is the 0-indexed character index into the input
/// string and exists so we can slice the original source cheaply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Pos {
    pub line: usize,
    pub column: usize,
    pub offset: usize,
}

impl Pos {
    pub const fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
        Self {
            start: pos,
            end: pos,
        }
    }

    /// Span covering both `self` and `other`. The result starts at the
    /// earlier of the two starts and ends at the later of the two ends.
    pub fn union(self, other: Span) -> Span {
        let start = if self.start.offset <= other.start.offset {
            self.start
        } else {
            other.start
        };
        let end = if self.end.offset >= other.end.offset {
            self.end
        } else {
            other.end
        };
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
            write!(
                f,
                "{}:{}-{}",
                self.start.line, self.start.column, self.end.column
            )
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
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}

/// RES-110 + RES-115: single-pass scan that records the byte
/// offset of each line start. Entry `0` is always byte `0` (BOF);
/// every subsequent entry is the byte immediately after a `\n`.
/// The returned `Vec` has no EOF sentinel — callers binary-search
/// it and use the length as the implicit upper bound.
///
/// Used by `pos_from_byte` below to convert byte offsets (from the
/// logos lexer or any byte-oriented scanner) to `Pos` values.
pub fn build_line_table(src: &str) -> Vec<usize> {
    let mut table: Vec<usize> = Vec::with_capacity(src.len() / 40 + 1);
    table.push(0);
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            table.push(i + 1);
        }
    }
    table
}

/// RES-110 + RES-115: O(log n) byte-offset → `Pos` conversion.
/// `table` must have been built via `build_line_table` over the
/// *same* source as `src`. `byte` may be any value in
/// `0..=src.len()`; out-of-range queries are clamped.
///
/// Note on signature: the RES-110 sketch was
/// `pos_from_byte(table, byte)` — the accompanying UTF-8 note
/// required counting *characters* from the start of the line,
/// which needs access to the source. We take `src` as a third
/// parameter to honour that note; the promised O(log n) behaviour
/// still applies to the line search over `table`. Character
/// counting inside the current line is O(line-length).
pub fn pos_from_byte(table: &[usize], src: &str, byte: usize) -> Pos {
    let byte = byte.min(src.len());
    let line_idx = match table.binary_search(&byte) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    };
    let line_start = table[line_idx];
    let line = line_idx + 1;
    let col_slice = src.get(line_start..byte).unwrap_or("");
    let column = col_slice.chars().count() + 1;
    let offset = src.get(..byte).map(|s| s.chars().count()).unwrap_or(0);
    Pos::new(line, column, offset)
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

    #[test]
    fn build_line_table_empty_source_has_single_bof_entry() {
        assert_eq!(build_line_table(""), vec![0]);
    }

    #[test]
    fn build_line_table_newlines_record_byte_after_each() {
        assert_eq!(build_line_table("abc\ndef\nghi"), vec![0, 4, 8]);
    }

    #[test]
    fn pos_from_byte_start_of_file() {
        let src = "abc\ndef";
        let table = build_line_table(src);
        assert_eq!(pos_from_byte(&table, src, 0), Pos::new(1, 1, 0));
    }

    #[test]
    fn pos_from_byte_respects_utf8_for_column() {
        // Each Greek letter is 2 bytes in UTF-8.
        let src = "αβγ";
        let table = build_line_table(src);
        let pos = pos_from_byte(&table, src, 2);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.column, 2);
        assert_eq!(pos.offset, 1);
    }

    // --- Union associativity and commutativity tests ---

    #[test]
    fn union_associativity_property() {
        // (a ∪ b) ∪ c == a ∪ (b ∪ c)
        let spans = [
            (
                Span::new(Pos::new(1, 1, 0), Pos::new(1, 3, 2)),
                Span::new(Pos::new(1, 5, 4), Pos::new(1, 7, 6)),
                Span::new(Pos::new(1, 9, 8), Pos::new(1, 11, 10)),
            ),
            (
                Span::new(Pos::new(1, 1, 0), Pos::new(2, 3, 10)),
                Span::new(Pos::new(1, 5, 4), Pos::new(3, 2, 20)),
                Span::new(Pos::new(2, 1, 5), Pos::new(2, 6, 15)),
            ),
        ];
        for (a, b, c) in spans.iter() {
            let left = a.union(*b).union(*c);
            let right = a.union(b.union(*c));
            assert_eq!(left, right, "associativity failed for {:?}", (a, b, c));
        }
    }

    #[test]
    fn union_commutativity_grid() {
        // a ∪ b == b ∪ a for various span combinations
        let test_cases = [
            (
                Span::new(Pos::new(1, 1, 0), Pos::new(1, 3, 2)),
                Span::new(Pos::new(1, 5, 4), Pos::new(1, 7, 6)),
            ),
            (
                Span::new(Pos::new(1, 1, 0), Pos::new(2, 3, 10)),
                Span::new(Pos::new(1, 5, 4), Pos::new(1, 8, 7)),
            ),
            (
                Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4)),
                Span::new(Pos::new(1, 2, 1), Pos::new(1, 4, 3)),
            ), // overlapping
            (
                Span::new(Pos::new(2, 5, 15), Pos::new(3, 2, 25)),
                Span::new(Pos::new(1, 1, 0), Pos::new(1, 10, 9)),
            ), // disjoint
        ];
        for (a, b) in test_cases.iter() {
            assert_eq!(
                a.union(*b),
                b.union(*a),
                "commutativity failed for {:?}",
                (a, b)
            );
        }
    }

    #[test]
    fn union_is_idempotent() {
        // a ∪ a == a
        let spans = [
            Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4)),
            Span::new(Pos::new(2, 3, 10), Pos::new(3, 2, 20)),
            Span::point(Pos::new(1, 1, 0)),
        ];
        for s in spans.iter() {
            let u = s.union(*s);
            assert_eq!(u, *s, "idempotence failed for {:?}", s);
        }
    }

    #[test]
    fn union_with_overlapping_spans() {
        // Two spans that partially overlap
        let a = Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4));
        let b = Span::new(Pos::new(1, 3, 2), Pos::new(1, 8, 7));
        let u = a.union(b);
        assert_eq!(u.start.offset, 0);
        assert_eq!(u.end.offset, 7);
        assert_eq!(u.len(), 7);
    }

    #[test]
    fn union_with_nested_spans() {
        // One span completely contains the other
        let outer = Span::new(Pos::new(1, 1, 0), Pos::new(2, 5, 20));
        let inner = Span::new(Pos::new(1, 5, 4), Pos::new(1, 8, 7));
        let u = outer.union(inner);
        assert_eq!(u, outer);
    }

    #[test]
    fn union_multiple_non_overlapping() {
        // Chain union of multiple non-overlapping spans
        let a = Span::new(Pos::new(1, 1, 0), Pos::new(1, 2, 1));
        let b = Span::new(Pos::new(1, 5, 4), Pos::new(1, 6, 5));
        let c = Span::new(Pos::new(1, 10, 9), Pos::new(1, 12, 11));
        let result = a.union(b).union(c);
        assert_eq!(result.start.offset, 0);
        assert_eq!(result.end.offset, 11);
    }

    // --- UTF-8 multibyte character tests ---

    #[test]
    fn pos_from_byte_with_4byte_emoji() {
        // 😀 is a 4-byte emoji (U+1F600)
        let src = "a😀b";
        let table = build_line_table(src);
        // 'a' is 1 byte, '😀' is 4 bytes, 'b' is 1 byte
        // byte 0: 'a', bytes 1-4: '😀', byte 5: 'b'
        let pos_after_emoji = pos_from_byte(&table, src, 5);
        assert_eq!(pos_after_emoji.line, 1);
        assert_eq!(pos_after_emoji.column, 3); // 'a' + '😀' + 1
        assert_eq!(pos_after_emoji.offset, 2); // 'a' + '😀'
    }

    #[test]
    fn pos_from_byte_with_cjk_3byte_chars() {
        // Chinese character '中' is 3 bytes (U+4E2D)
        let src = "中国";
        let table = build_line_table(src);
        // '中' is bytes 0-2, '国' is bytes 3-5
        let pos_after_first = pos_from_byte(&table, src, 3);
        assert_eq!(pos_after_first.line, 1);
        assert_eq!(pos_after_first.column, 2);
        assert_eq!(pos_after_first.offset, 1);
    }

    #[test]
    fn pos_from_byte_with_accented_2byte_chars() {
        // Accented 'é' is 2 bytes (U+00E9)
        let src = "café";
        let table = build_line_table(src);
        // 'c' = 1 byte, 'a' = 1 byte, 'f' = 1 byte, 'é' = 2 bytes
        // Bytes: c(0), a(1), f(2), é(3-4), (total 5 bytes)
        let pos_at_e = pos_from_byte(&table, src, 3);
        assert_eq!(pos_at_e.line, 1);
        assert_eq!(pos_at_e.column, 4);
        assert_eq!(pos_at_e.offset, 3);
        let pos_after_e = pos_from_byte(&table, src, 5);
        assert_eq!(pos_after_e.column, 5);
        assert_eq!(pos_after_e.offset, 4);
    }

    #[test]
    fn pos_from_byte_mixed_multibyte_chars() {
        // Mix of different UTF-8 char widths: é(2) + 中(3) + 😀(4)
        let src = "é中😀";
        let table = build_line_table(src);
        // é: bytes 0-1, 中: bytes 2-4, 😀: bytes 5-8
        let pos_after_e = pos_from_byte(&table, src, 2);
        assert_eq!(pos_after_e.offset, 1);
        let pos_after_zhong = pos_from_byte(&table, src, 5);
        assert_eq!(pos_after_zhong.offset, 2);
        let pos_after_emoji = pos_from_byte(&table, src, 9);
        assert_eq!(pos_after_emoji.offset, 3);
    }

    // --- Line ending tests ---

    #[test]
    fn pos_from_byte_with_crlf_line_endings() {
        // CRLF line endings: "abc\r\ndef\r\nghi"
        let src = "abc\r\ndef\r\nghi";
        let table = build_line_table(src);
        // Line 1: bytes 0-4 ("abc\r\n")
        // Line 2: bytes 5-9 ("def\r\n")
        // Line 3: bytes 10-12 ("ghi")
        assert_eq!(table, vec![0, 5, 10]);
        let pos_line2_start = pos_from_byte(&table, src, 5);
        assert_eq!(pos_line2_start.line, 2);
        assert_eq!(pos_line2_start.column, 1);
        let pos_line2_mid = pos_from_byte(&table, src, 6);
        assert_eq!(pos_line2_mid.line, 2);
        assert_eq!(pos_line2_mid.column, 2);
    }

    #[test]
    fn pos_from_byte_with_mixed_line_endings() {
        // Mix of LF and CRLF: "a\nb\r\nc"
        let src = "a\nb\r\nc";
        let table = build_line_table(src);
        // Line 1: "a\n" (bytes 0-1, table records byte 2 as start of line 2)
        // Line 2: "b\r\n" (bytes 2-4, table records byte 5 as start of line 3)
        // Line 3: "c" (byte 5)
        assert_eq!(table, vec![0, 2, 5]);
    }

    #[test]
    fn pos_from_byte_at_eof() {
        let src = "line1\nline2";
        let table = build_line_table(src);
        let pos_eof = pos_from_byte(&table, src, src.len());
        assert_eq!(pos_eof.line, 2);
        assert_eq!(pos_eof.offset, src.chars().count());
    }

    #[test]
    fn pos_from_byte_clamping_beyond_eof() {
        let src = "hello";
        let table = build_line_table(src);
        let pos_way_beyond = pos_from_byte(&table, src, 1000);
        let pos_eof = pos_from_byte(&table, src, src.len());
        assert_eq!(pos_way_beyond, pos_eof);
    }

    // --- Line boundary tests ---

    #[test]
    fn pos_from_byte_at_line_starts() {
        let src = "line1\nline2\nline3";
        let table = build_line_table(src);
        assert_eq!(table, vec![0, 6, 12]);
        // Position at each line start
        let pos_line1 = pos_from_byte(&table, src, 0);
        assert_eq!(pos_line1.line, 1);
        assert_eq!(pos_line1.column, 1);
        let pos_line2 = pos_from_byte(&table, src, 6);
        assert_eq!(pos_line2.line, 2);
        assert_eq!(pos_line2.column, 1);
        let pos_line3 = pos_from_byte(&table, src, 12);
        assert_eq!(pos_line3.line, 3);
        assert_eq!(pos_line3.column, 1);
    }

    #[test]
    fn pos_from_byte_at_line_ends() {
        let src = "abc\ndef\nghi";
        let table = build_line_table(src);
        // Byte 3 is at the '\n' of line 1
        let pos_newline1 = pos_from_byte(&table, src, 3);
        assert_eq!(pos_newline1.line, 1);
        assert_eq!(pos_newline1.column, 4);
    }

    // --- Span properties and ordering ---

    #[test]
    fn span_length_consistency() {
        let test_spans = [
            (Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4)), 4),
            (Span::new(Pos::new(1, 1, 0), Pos::new(2, 3, 20)), 20),
            (Span::point(Pos::new(1, 1, 0)), 0),
            (Span::new(Pos::new(5, 10, 100), Pos::new(5, 10, 100)), 0),
        ];
        for (span, expected_len) in test_spans.iter() {
            assert_eq!(
                span.len(),
                *expected_len,
                "span {:?} has wrong length",
                span
            );
        }
    }

    #[test]
    fn span_is_empty_consistency() {
        let zero_width = Span::new(Pos::new(1, 1, 0), Pos::new(1, 1, 0));
        assert!(zero_width.is_empty());
        assert_eq!(zero_width.len(), 0);

        let single_char = Span::new(Pos::new(1, 1, 0), Pos::new(1, 2, 1));
        assert!(!single_char.is_empty());
        assert_eq!(single_char.len(), 1);
    }

    #[test]
    fn point_spans_are_empty_by_construction() {
        let points = [
            Span::point(Pos::new(1, 1, 0)),
            Span::point(Pos::new(5, 10, 50)),
            Span::point(Pos::new(100, 1, 999)),
        ];
        for point in points.iter() {
            assert!(point.is_empty());
            assert_eq!(point.len(), 0);
            assert_eq!(point.start, point.end);
        }
    }

    #[test]
    fn span_ordering_by_offset() {
        let a = Span::new(Pos::new(1, 1, 0), Pos::new(1, 5, 4));
        let b = Span::new(Pos::new(1, 10, 9), Pos::new(1, 15, 14));
        // a starts before b
        assert!(a.start.offset < b.start.offset);
        // union respects this ordering
        let union_ab = a.union(b);
        assert_eq!(union_ab.start.offset, a.start.offset);
        assert_eq!(union_ab.end.offset, b.end.offset);
    }

    #[test]
    fn pos_line_and_offset_monotonicity() {
        let src = "a\nbb\nccc\ndddd";
        let table = build_line_table(src);
        // Positions should increase monotonically as byte offset increases
        let mut prev_pos = pos_from_byte(&table, src, 0);
        for byte in 1..=src.len() {
            let pos = pos_from_byte(&table, src, byte);
            assert!(
                pos.line >= prev_pos.line,
                "line decreased: {:?} -> {:?}",
                prev_pos,
                pos
            );
            if pos.line == prev_pos.line {
                assert!(
                    pos.column >= prev_pos.column,
                    "column decreased on same line: {:?} -> {:?}",
                    prev_pos,
                    pos
                );
            }
            prev_pos = pos;
        }
    }

    #[test]
    fn empty_source_edge_cases() {
        let src = "";
        let table = build_line_table(src);
        assert_eq!(table, vec![0]);
        let pos = pos_from_byte(&table, src, 0);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.column, 1);
        assert_eq!(pos.offset, 0);
    }
}
