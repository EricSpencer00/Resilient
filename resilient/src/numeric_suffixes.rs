//! RES-2616: Numeric literal type suffixes (`42u8`, `3.14f32`).
//!
//! Allows integers and floats to carry a type suffix that pins the value
//! to a specific width without an explicit cast. The parser recognises
//! the suffix immediately after an integer or float token and desugars
//! it to a `CallExpression` wrapping the appropriate cast builtin:
//!
//! ```text
//! 42u8       →  as_int8(42)
//! 255u8      →  as_int8(255)
//! 0xFFu32    →  as_uint32(0xFF)   (hex + suffix)
//! 3.14f32    →  as_f32(3.14)
//! ```
//!
//! # Overflow behaviour
//!
//! The cast builtins (`as_int8`, `as_uint8`, etc.) wrap on overflow,
//! matching the semantics of numeric narrowing in Rust. Parse-time range
//! validation is a follow-up (RES-2616 acceptance criterion "Overflow
//! at parse time") tracked separately.
//!
//! # Feature isolation
//!
//! All logic lives here. `lib.rs` adds only `mod numeric_suffixes;`
//! and the two parse-arm patches in `parse_expression`.

use crate::Node;
use crate::span::Span;

/// A recognised numeric type suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NumericSuffix {
    // Signed integer widths.
    I8,
    I16,
    I32,
    I64,
    // Unsigned integer widths.
    U8,
    U16,
    U32,
    U64,
    // Floating-point widths.
    F32,
    F64,
}

impl NumericSuffix {
    /// Returns the name of the cast builtin that implements this suffix.
    pub(crate) fn builtin_name(self) -> &'static str {
        match self {
            NumericSuffix::I8 => "as_int8",
            NumericSuffix::I16 => "as_int16",
            NumericSuffix::I32 => "as_int32",
            NumericSuffix::I64 => "to_int",
            NumericSuffix::U8 => "as_uint8",
            NumericSuffix::U16 => "as_uint16",
            NumericSuffix::U32 => "as_uint32",
            NumericSuffix::U64 => "as_uint64",
            NumericSuffix::F32 => "as_f32",
            NumericSuffix::F64 => "to_float",
        }
    }

    /// Returns `true` if this suffix can follow an integer literal.
    #[allow(dead_code)] // only used in tests; kept for documentation
    pub(crate) fn is_int(self) -> bool {
        !matches!(self, NumericSuffix::F32 | NumericSuffix::F64)
    }

    /// Returns `true` if this suffix can follow a float literal.
    pub(crate) fn is_float(self) -> bool {
        matches!(self, NumericSuffix::F32 | NumericSuffix::F64)
    }
}

/// Try to parse a type suffix from a bare identifier name.
///
/// Returns `None` for any string that is not a recognised numeric suffix.
/// Both lowercase Rust-style (`u8`, `i32`) and PascalCase Resilient-style
/// (`UInt8`, `Int32`) forms are recognised.
pub(crate) fn try_parse_suffix(ident: &str) -> Option<NumericSuffix> {
    match ident {
        "i8" | "Int8" => Some(NumericSuffix::I8),
        "i16" | "Int16" => Some(NumericSuffix::I16),
        "i32" | "Int32" => Some(NumericSuffix::I32),
        "i64" | "Int64" | "Int" => Some(NumericSuffix::I64),
        "u8" | "UInt8" => Some(NumericSuffix::U8),
        "u16" | "UInt16" => Some(NumericSuffix::U16),
        "u32" | "UInt32" => Some(NumericSuffix::U32),
        "u64" | "UInt64" => Some(NumericSuffix::U64),
        "f32" | "Float32" => Some(NumericSuffix::F32),
        "f64" | "Float64" | "Float" => Some(NumericSuffix::F64),
        _ => None,
    }
}

/// Desugar an integer literal with a type suffix into a `CallExpression`.
///
/// `42u8` → `as_int8(42)` (i.e. `Node::CallExpression { function:
/// Identifier("as_int8"), arguments: [IntegerLiteral(42)] }`).
pub(crate) fn desugar_int_with_suffix(value: i64, suffix: NumericSuffix, span: Span) -> Node {
    let builtin = suffix.builtin_name();
    Node::CallExpression {
        function: Box::new(Node::Identifier {
            name: builtin.to_string(),
            span,
        }),
        arguments: vec![Node::IntegerLiteral { value, span }],
        span,
        // Named arguments are not used here.
    }
}

/// Desugar a float literal with a type suffix into a `CallExpression`.
///
/// `3.14f32` → `as_f32(3.14)` (i.e. `Node::CallExpression { ... }`).
pub(crate) fn desugar_float_with_suffix(value: f64, suffix: NumericSuffix, span: Span) -> Node {
    let builtin = suffix.builtin_name();
    Node::CallExpression {
        function: Box::new(Node::Identifier {
            name: builtin.to_string(),
            span,
        }),
        arguments: vec![Node::FloatLiteral { value, span }],
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_program;

    // ── try_parse_suffix ─────────────────────────────────────────────────────

    #[test]
    fn known_suffixes_are_recognised() {
        for (s, expected) in [
            ("i8", NumericSuffix::I8),
            ("i16", NumericSuffix::I16),
            ("i32", NumericSuffix::I32),
            ("i64", NumericSuffix::I64),
            ("u8", NumericSuffix::U8),
            ("u16", NumericSuffix::U16),
            ("u32", NumericSuffix::U32),
            ("u64", NumericSuffix::U64),
            ("f32", NumericSuffix::F32),
            ("f64", NumericSuffix::F64),
        ] {
            assert_eq!(try_parse_suffix(s), Some(expected), "suffix {s}");
        }
    }

    #[test]
    fn pascal_case_aliases_recognised() {
        assert_eq!(try_parse_suffix("UInt8"), Some(NumericSuffix::U8));
        assert_eq!(try_parse_suffix("Int32"), Some(NumericSuffix::I32));
        assert_eq!(try_parse_suffix("Float32"), Some(NumericSuffix::F32));
    }

    #[test]
    fn unknown_identifiers_return_none() {
        assert!(try_parse_suffix("hello").is_none());
        assert!(try_parse_suffix("x").is_none());
        assert!(try_parse_suffix("u128").is_none()); // not in the set
        assert!(try_parse_suffix("").is_none());
    }

    #[test]
    fn is_int_and_is_float_agree() {
        assert!(NumericSuffix::U8.is_int());
        assert!(!NumericSuffix::U8.is_float());
        assert!(NumericSuffix::F32.is_float());
        assert!(!NumericSuffix::F32.is_int());
    }

    // ── end-to-end parser/interpreter tests ──────────────────────────────────

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    #[test]
    fn u8_suffix_literal() {
        let r = run("println(255u8);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("255"), "stdout: {}", r.stdout);
    }

    #[test]
    fn i16_suffix_literal() {
        let r = run("println(1000i16);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("1000"), "stdout: {}", r.stdout);
    }

    #[test]
    fn u32_suffix_literal() {
        let r = run("println(0u32);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn f32_suffix_on_float() {
        let r = run("println(3.14f32);");
        assert!(r.ok, "errors: {:?}", r.errors);
        // f32 precision truncates 3.14 to 3.14000010...
        assert!(r.stdout.contains("3.14"), "stdout: {}", r.stdout);
    }

    #[test]
    fn f64_suffix_on_float() {
        let r = run("println(1.5f64);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("1.5"), "stdout: {}", r.stdout);
    }

    #[test]
    fn i32_suffix_in_expression() {
        let r = run("let x = 10i32; let y = 20i32; println(x + y);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("30"), "stdout: {}", r.stdout);
    }

    #[test]
    fn u8_assigned_to_typed_variable() {
        let r = run("let x: UInt8 = 200u8; println(x);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("200"), "stdout: {}", r.stdout);
    }

    #[test]
    fn f32_suffix_on_integer_literal() {
        // 42f32 should work — the int is first cast by as_f32
        let r = run("println(42f32);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("42"), "stdout: {}", r.stdout);
    }

    #[test]
    fn hex_literal_with_u32_suffix() {
        // 0xFF is a HexInt; the u32 suffix should still be consumed
        let r = run("println(0xFFu32);");
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("255"), "stdout: {}", r.stdout);
    }
}
