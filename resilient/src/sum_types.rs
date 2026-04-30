//! RES-400: sum types — tagged enum declarations + match.
//!
//! ## Surface syntax (target)
//!
//! ```text
//! enum Color { Red, Green, Blue }
//!
//! enum Shape {
//!     Circle { r: float },
//!     Square { side: float },
//!     Rect { w: float, h: float },
//! }
//!
//! fn area(s: Shape) -> float {
//!     match s {
//!         Shape::Circle { r } => 3.14 * r * r,
//!         Shape::Square { side } => side * side,
//!         Shape::Rect { w, h } => w * h,
//!     }
//! }
//! ```
//!
//! ## What ships in this PR (PR 1 of N)
//!
//! Just the *parser scaffold* for payload-less variants:
//!
//! ```text
//! enum Color { Red, Green, Blue }
//! ```
//!
//! Specifically:
//!
//! * Lexer: `enum` → `Token::Enum` keyword.
//! * Parser: `parse_enum_decl` recognises the declaration and produces
//!   a `Node::EnumDecl { name, variants, span }` with `EnumVariant`
//!   entries (name + span only — no payload kinds yet).
//! * Validation: empty `enum` body and duplicate variant names are
//!   reported as parser errors with file:line:col diagnostics.
//!
//! ## What's deferred (subsequent PRs in the chain)
//!
//! * **PR 2**: payload variants (named-field structs and tuple-style),
//!   typechecker registration of the type and its variants.
//! * **PR 3**: pattern matching against variants — extends the
//!   existing `match` syntax to recognise `EnumName::Variant { ... }`
//!   patterns.
//! * **PR 4**: exhaustiveness check — a `match` missing any variant
//!   becomes a compile-time error with the list of missing variants.
//! * **PR 5**: interpreter eval for constructor expressions and
//!   variant matching.
//! * **PR 6**: re-implement `Option` / `Result` on top of this
//!   machinery (the issue notes this is a follow-up).
//!
//! ## Where things live
//!
//! Per the feature-isolation pattern in `CLAUDE.md`:
//! * All sum-type *logic* lives here.
//! * `lib.rs` only carries the `Token::Enum` arm in `<EXTENSION_TOKENS>`,
//!   the keyword mapping in `<EXTENSION_KEYWORDS>`, the `mod sum_types;`
//!   declaration, and the dispatch arm
//!   `Token::Enum => Some(crate::sum_types::parse_enum_decl(self))`
//!   in the top-level item-parsing loop.

use crate::{EnumField, EnumPayload, EnumVariant, Node, Parser, Token};
use std::collections::HashSet;

/// Parse an `enum Name { Variant1, Variant2, ... }` declaration.
///
/// Called from the top-level item-parsing dispatch when the parser
/// sees `Token::Enum`. The function consumes from the `enum` keyword
/// through (and including) the closing `}`.
///
/// Errors are reported via `parser.record_error` with file:line:col
/// — the function tries to recover so subsequent items continue
/// parsing rather than the whole program aborting at the first
/// malformed enum.
pub(crate) fn parse_enum_decl(parser: &mut Parser) -> Node {
    let enum_span = parser.span_at_current();
    parser.next_token(); // consume 'enum'

    let name = match &parser.current_token {
        Token::Identifier(n) => n.clone(),
        other => {
            let tok = other.clone();
            parser.record_error(format!("Expected identifier after 'enum', found {}", tok));
            String::new()
        }
    };
    parser.next_token(); // consume name (or whatever was there)

    if parser.current_token != Token::LeftBrace {
        let tok = parser.current_token.clone();
        parser.record_error(format!(
            "Expected '{{' after 'enum {}', found {}",
            name, tok
        ));
        // Best-effort recovery: return an empty enum so the rest of
        // the program still parses.
        return Node::EnumDecl {
            name,
            variants: Vec::new(),
            span: enum_span,
        };
    }
    parser.next_token(); // consume '{'

    let mut variants: Vec<EnumVariant> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    loop {
        match &parser.current_token {
            Token::RightBrace => {
                // Leave the closing `}` on `current_token` so the
                // outer statement-loop's bookkeeping (which expects
                // a declaration to terminate on its closing brace,
                // not one token past) remains correct. Matches
                // `parse_struct_decl_with_attrs`'s convention.
                break;
            }
            Token::Eof => {
                parser.record_error(format!(
                    "Unexpected end of input inside 'enum {}' body — expected '}}'",
                    name
                ));
                break;
            }
            Token::Identifier(variant_name) => {
                let v_name = variant_name.clone();
                let v_span = parser.span_at_current();
                parser.next_token(); // consume variant name

                // PR 2: parse the optional payload. `{ … }` for
                // named-field, `( … )` for tuple-style, otherwise
                // `EnumPayload::None`.
                let payload = match &parser.current_token {
                    Token::LeftBrace => parse_named_payload(parser, &name, &v_name),
                    Token::LeftParen => parse_tuple_payload(parser, &name, &v_name),
                    _ => EnumPayload::None,
                };

                if !seen.insert(v_name.clone()) {
                    parser
                        .record_error(format!("Duplicate variant '{}' in 'enum {}'", v_name, name));
                } else {
                    variants.push(EnumVariant {
                        name: v_name,
                        span: v_span,
                        payload,
                    });
                }

                // Trailing comma is optional — `enum X { A }`,
                // `enum X { A, }`, and `enum X { A, B }` all parse.
                if parser.current_token == Token::Comma {
                    parser.next_token();
                }
            }
            other => {
                let tok = other.clone();
                parser.record_error(format!(
                    "Expected variant name in 'enum {}', found {}",
                    name, tok
                ));
                // Recovery: advance one token so we don't loop
                // forever on the same bad input.
                parser.next_token();
            }
        }
    }

    if variants.is_empty() {
        parser.record_error(format!(
            "'enum {}' has no variants — at least one variant is required",
            name
        ));
    }

    Node::EnumDecl {
        name,
        variants,
        span: enum_span,
    }
}

/// Recovery helper: advance until the next `,` or `}` (or EOF) so the
/// outer loop can resume parsing later variants. Consumes the `,` if
/// present so the next iteration starts on the variant after the
/// malformed one.
#[allow(dead_code)]
fn skip_to_variant_separator(parser: &mut Parser) {
    while parser.current_token != Token::Comma
        && parser.current_token != Token::RightBrace
        && parser.current_token != Token::Eof
    {
        parser.next_token();
    }
    if parser.current_token == Token::Comma {
        parser.next_token();
    }
}

/// RES-400 PR 2: parse a named-field payload `{ field: Type, field: Type }`.
///
/// Called when the variant-name dispatch sees `Token::LeftBrace`.
/// Consumes from the `{` through (and including) the closing `}`.
/// Errors are reported via `parser.record_error` with file:line:col;
/// recovery skips to the next `,` or `}` so the rest of the enum
/// continues parsing.
fn parse_named_payload(parser: &mut Parser, enum_name: &str, variant_name: &str) -> EnumPayload {
    parser.next_token(); // consume '{'
    let mut fields: Vec<EnumField> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    loop {
        match &parser.current_token {
            Token::RightBrace => {
                parser.next_token(); // consume '}'
                return EnumPayload::Named(fields);
            }
            Token::Eof => {
                parser.record_error(format!(
                    "Unexpected end of input inside payload for '{}::{}' — expected '}}'",
                    enum_name, variant_name
                ));
                return EnumPayload::Named(fields);
            }
            Token::Identifier(field_name) => {
                let f_name = field_name.clone();
                let f_span = parser.span_at_current();
                parser.next_token(); // consume field name
                if parser.current_token != Token::Colon {
                    let tok = parser.current_token.clone();
                    parser.record_error(format!(
                        "Expected ':' after field name '{}::{}.{}', found {}",
                        enum_name, variant_name, f_name, tok
                    ));
                    skip_to_payload_separator(parser, Token::RightBrace);
                    continue;
                }
                parser.next_token(); // consume ':'
                let ty = match parse_payload_type(parser) {
                    Some(t) => t,
                    None => {
                        parser.record_error(format!(
                            "Expected type after ':' in field '{}::{}.{}'",
                            enum_name, variant_name, f_name
                        ));
                        skip_to_payload_separator(parser, Token::RightBrace);
                        continue;
                    }
                };
                if !seen.insert(f_name.clone()) {
                    parser.record_error(format!(
                        "Duplicate field '{}' in variant '{}::{}'",
                        f_name, enum_name, variant_name
                    ));
                } else {
                    fields.push(EnumField {
                        name: f_name,
                        ty,
                        span: f_span,
                    });
                }
                if parser.current_token == Token::Comma {
                    parser.next_token();
                }
            }
            other => {
                let tok = other.clone();
                parser.record_error(format!(
                    "Expected field name in payload for '{}::{}', found {}",
                    enum_name, variant_name, tok
                ));
                parser.next_token();
            }
        }
    }
}

/// RES-400 PR 2: parse a tuple-style payload `( Type, Type, … )`.
///
/// Called when the variant-name dispatch sees `Token::LeftParen`.
/// Consumes from the `(` through (and including) the closing `)`.
fn parse_tuple_payload(parser: &mut Parser, enum_name: &str, variant_name: &str) -> EnumPayload {
    parser.next_token(); // consume '('
    let mut tys: Vec<String> = Vec::new();
    loop {
        match &parser.current_token {
            Token::RightParen => {
                parser.next_token(); // consume ')'
                return EnumPayload::Tuple(tys);
            }
            Token::Eof => {
                parser.record_error(format!(
                    "Unexpected end of input inside payload for '{}::{}' — expected ')'",
                    enum_name, variant_name
                ));
                return EnumPayload::Tuple(tys);
            }
            _ => {
                let ty = match parse_payload_type(parser) {
                    Some(t) => t,
                    None => {
                        let tok = parser.current_token.clone();
                        parser.record_error(format!(
                            "Expected type in payload for '{}::{}', found {}",
                            enum_name, variant_name, tok
                        ));
                        skip_to_payload_separator(parser, Token::RightParen);
                        continue;
                    }
                };
                tys.push(ty);
                if parser.current_token == Token::Comma {
                    parser.next_token();
                }
            }
        }
    }
}

/// Recovery helper for malformed payloads: advance until the next
/// `,` or the supplied closing token (or EOF). Consumes the `,` if
/// present so the next field/type starts cleanly.
fn skip_to_payload_separator(parser: &mut Parser, close: Token) {
    while parser.current_token != Token::Comma
        && parser.current_token != close
        && parser.current_token != Token::Eof
    {
        parser.next_token();
    }
    if parser.current_token == Token::Comma {
        parser.next_token();
    }
}

/// Parse a single type reference for use in a variant payload.
/// Accepts an identifier (covers primitives like `int`/`float`/
/// `string` and user-defined names). Returns the type name as a
/// string per the AST's "single-string type" convention; `None` if
/// the current token isn't a type name.
fn parse_payload_type(parser: &mut Parser) -> Option<String> {
    match &parser.current_token {
        Token::Identifier(n) => {
            let name = n.clone();
            parser.next_token();
            Some(name)
        }
        _ => None,
    }
}

/// RES-400 PR 1: helper used by `lib.rs` (and tests) to unwrap an
/// `EnumDecl` from a parsed `Node::Program`. Behind a `cfg(test)`
/// gate today; later PRs will use it from the typechecker / repr
/// layout passes too.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn extract_enum_decls(program: &Node) -> Vec<&Node> {
    match program {
        Node::Program(stmts) => stmts
            .iter()
            .map(|s| &s.node)
            .filter(|n| matches!(n, Node::EnumDecl { .. }))
            .collect(),
        _ => Vec::new(),
    }
}

/// Helper for tests: extract just the variant names of an EnumDecl,
/// in declaration order.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn variant_names(decl: &Node) -> Option<Vec<String>> {
    match decl {
        Node::EnumDecl { variants, .. } => Some(variants.iter().map(|v| v.name.clone()).collect()),
        _ => None,
    }
}

/// Helper for tests: extract the enum's name.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn enum_name(decl: &Node) -> Option<&str> {
    match decl {
        Node::EnumDecl { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn parses_payload_less_enum() {
        let (program, errs) = parse("enum Color { Red, Green, Blue }");
        assert!(errs.is_empty(), "expected clean parse, got: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        assert_eq!(decls.len(), 1);
        assert_eq!(super::enum_name(decls[0]), Some("Color"));
        assert_eq!(
            super::variant_names(decls[0]).unwrap(),
            vec!["Red", "Green", "Blue"]
        );
    }

    #[test]
    fn parses_single_variant_enum() {
        let (program, errs) = parse("enum Just { One }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        assert_eq!(super::variant_names(decls[0]).unwrap(), vec!["One"]);
    }

    #[test]
    fn accepts_trailing_comma() {
        let (program, errs) = parse("enum Color { Red, Green, Blue, }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        assert_eq!(
            super::variant_names(decls[0]).unwrap(),
            vec!["Red", "Green", "Blue"]
        );
    }

    #[test]
    fn empty_enum_body_is_an_error() {
        let (_, errs) = parse("enum Empty {}");
        assert!(
            errs.iter().any(|e| e.contains("no variants")),
            "expected 'no variants' error, got: {:?}",
            errs
        );
    }

    #[test]
    fn missing_open_brace_is_an_error() {
        let (_, errs) = parse("enum Bad");
        assert!(
            errs.iter().any(|e| e.contains("Expected '{'")),
            "expected '{{' error, got: {:?}",
            errs
        );
    }

    #[test]
    fn duplicate_variant_is_an_error() {
        let (_, errs) = parse("enum Dup { Red, Red }");
        assert!(
            errs.iter().any(|e| e.contains("Duplicate variant")),
            "expected duplicate-variant error, got: {:?}",
            errs
        );
    }

    #[test]
    fn parses_named_field_payload() {
        let (program, errs) =
            parse("enum Shape { Circle { r: float }, Rect { w: float, h: float } }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        let v = match decls[0] {
            crate::Node::EnumDecl { variants, .. } => variants,
            _ => panic!(),
        };
        assert_eq!(v.len(), 2);
        match &v[0].payload {
            crate::EnumPayload::Named(fields) => {
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "r");
                assert_eq!(fields[0].ty, "float");
            }
            other => panic!("expected Named payload, got {:?}", other),
        }
        match &v[1].payload {
            crate::EnumPayload::Named(fields) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "w");
                assert_eq!(fields[0].ty, "float");
                assert_eq!(fields[1].name, "h");
                assert_eq!(fields[1].ty, "float");
            }
            other => panic!("expected Named payload, got {:?}", other),
        }
    }

    #[test]
    fn parses_tuple_payload() {
        let (program, errs) = parse("enum Pair { Just(int), Both(int, int) }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        let v = match decls[0] {
            crate::Node::EnumDecl { variants, .. } => variants,
            _ => panic!(),
        };
        match &v[0].payload {
            crate::EnumPayload::Tuple(tys) => {
                assert_eq!(tys, &vec!["int".to_string()]);
            }
            other => panic!("expected Tuple payload, got {:?}", other),
        }
        match &v[1].payload {
            crate::EnumPayload::Tuple(tys) => {
                assert_eq!(tys, &vec!["int".to_string(), "int".to_string()]);
            }
            other => panic!("expected Tuple payload, got {:?}", other),
        }
    }

    #[test]
    fn payload_less_variants_carry_none_payload() {
        let (program, errs) = parse("enum Color { Red, Green, Blue }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        let v = match decls[0] {
            crate::Node::EnumDecl { variants, .. } => variants,
            _ => panic!(),
        };
        for variant in v {
            match &variant.payload {
                crate::EnumPayload::None => {}
                other => panic!("expected None payload, got {:?}", other),
            }
        }
    }

    #[test]
    fn mixed_payload_kinds_in_one_enum() {
        let (program, errs) = parse("enum Shape { Empty, Point(int, int), Circle { r: float } }");
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let decls = super::extract_enum_decls(&program);
        let v = match decls[0] {
            crate::Node::EnumDecl { variants, .. } => variants,
            _ => panic!(),
        };
        assert!(matches!(v[0].payload, crate::EnumPayload::None));
        assert!(matches!(v[1].payload, crate::EnumPayload::Tuple(_)));
        assert!(matches!(v[2].payload, crate::EnumPayload::Named(_)));
    }

    #[test]
    fn duplicate_field_in_named_payload_is_an_error() {
        let (_, errs) = parse("enum Bad { Var { x: int, x: int } }");
        assert!(
            errs.iter().any(|e| e.contains("Duplicate field")),
            "expected duplicate-field error, got: {:?}",
            errs
        );
    }

    #[test]
    fn missing_colon_in_named_field_is_an_error() {
        let (_, errs) = parse("enum Bad { Var { x int } }");
        assert!(
            errs.iter().any(|e| e.contains("Expected ':'")),
            "expected colon error, got: {:?}",
            errs
        );
    }

    #[test]
    fn enum_does_not_swallow_following_items() {
        // Recovery test: after parsing an enum, the parser should
        // continue with whatever comes next. We only assert that
        // the program contains both an EnumDecl and at least one
        // additional top-level node — the exact shape of the
        // following item depends on parser detail (top-level
        // `let y = 5;` happens to come back as Assignment in the
        // current grammar; what matters here is that the parser
        // didn't choke on the `}` boundary).
        let (program, errs) = parse(
            r#"
            enum Color { Red, Green }
            let y = 5;
            "#,
        );
        assert!(errs.is_empty(), "errs: {:?}", errs);
        let enum_decls = super::extract_enum_decls(&program);
        assert_eq!(enum_decls.len(), 1);
        match &program {
            crate::Node::Program(stmts) => {
                assert!(
                    stmts.len() >= 2,
                    "expected enum + something else; got {} stmts: {:#?}",
                    stmts.len(),
                    stmts
                );
                // Whatever the second statement is, it must NOT be
                // another EnumDecl — the regression we're guarding
                // against is the enum parser consuming subsequent
                // tokens.
                assert!(
                    !matches!(&stmts[1].node, crate::Node::EnumDecl { .. }),
                    "second statement was unexpectedly another EnumDecl"
                );
            }
            _ => panic!("expected Program node"),
        }
    }
}
