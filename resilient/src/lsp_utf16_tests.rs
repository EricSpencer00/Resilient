use super::*;
use crate::span::{Pos, Span};
use tower_lsp::lsp_types::Position;

#[test]
fn parser_diagnostic_maps_scalar_column_to_utf16() {
    let (range, message) = extract_range_and_message_in_source("1:4: unexpected token", "😀abc\n");
    assert_eq!(message, "unexpected token");
    assert_eq!(range.start, Position::new(0, 4));
    assert_eq!(range.end, Position::new(0, 4));
}

#[test]
fn typechecker_diagnostic_maps_scalar_column_to_utf16() {
    let (range, message) =
        extract_range_and_message_in_source("file.rz:1:4: type mismatch", "😀abc\n");
    assert_eq!(message, "type mismatch");
    assert_eq!(range.start, Position::new(0, 4));
    assert_eq!(range.end, Position::new(0, 4));
}

#[test]
fn source_span_maps_non_bmp_prefix_to_utf16() {
    let span = Span::new(Pos::new(1, 2, 1), Pos::new(1, 7, 6));
    let range = span_to_range_in_source("\"😀name\"", span);
    assert_eq!(range.start, Position::new(0, 1));
    assert_eq!(range.end, Position::new(0, 7));
}

#[test]
fn identifier_at_uses_utf16_cursor_and_range() {
    let result = identifier_at("\"😀\" value", Position::new(0, 5));
    let (name, range) = result.expect("expected identifier hit");
    assert_eq!(name, "value");
    assert_eq!(range.start, Position::new(0, 5));
    assert_eq!(range.end, Position::new(0, 10));
}

#[test]
fn hover_literal_at_uses_utf16_cursor_and_range() {
    let result = hover_literal_at("\"😀\" 1", Position::new(0, 5));
    let (type_name, range) = result.expect("expected literal hover");
    assert_eq!(type_name, "Int");
    assert_eq!(range.start, Position::new(0, 5));
    assert_eq!(range.end, Position::new(0, 6));
}

#[test]
fn invalid_source_position_falls_back_to_scalar_column() {
    assert_eq!(lsp_position_for_source("", 12, 7), Position::new(11, 6));
}
