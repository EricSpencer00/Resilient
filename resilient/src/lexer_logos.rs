//! RES-108: logos-based lexer (G5 foundation, behind the
//! `logos-lexer` feature flag).
//!
//! This module defines a `#[derive(Logos)]` token enum that mirrors
//! every variant the hand-rolled scanner in `main.rs` produces, and
//! exposes `tokenize` — the entry point `Lexer::new` reaches for when
//! the `logos-lexer` feature is enabled.
//!
//! The legacy hand-rolled lexer stays authoritative until RES-109
//! benchmarks land. Parity is enforced by the `lexer_parity` unit
//! tests: they scan every example in `resilient/examples/` through
//! both code paths and assert the resulting `(Token, Span)` streams
//! are identical.
//!
//! Design notes:
//! - Keyword `#[token]` attributes intentionally precede the generic
//!   identifier `#[regex]` so logos resolves them via its length
//!   priority. `_` alone maps to `Token::Underscore`; `_foo` / `foo_`
//!   become identifiers.
//! - Numeric literals accept `_` digit separators inside hex / binary
//!   bodies, matching the hand-rolled lexer's `read_radix_number`.
//! - String literals go through the `string_lit` callback so escape
//!   sequences (`\n`, `\t`, `\r`, `\\`, `\"`) are expanded in place
//!   and unknown escapes are preserved as backslash + char, exactly
//!   as the legacy `read_string` does.
//! - Block comments use the `block_comment` callback instead of a
//!   skip-regex because writing the non-nesting C comment pattern in
//!   logos's regex flavour is fiddly; the callback scans forward to
//!   `*/` (or EOF) and returns `logos::Skip`.

use logos::Logos;

use crate::Token;
use crate::pos_from_byte;
use crate::span::{Pos, Span};

/// Token variants the logos derive produces. Converted to the
/// crate-level `Token` in `convert` below.
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\n\r\f]+")]
#[logos(skip r"//[^\n]*")]
enum Tok {
    // --- two-char operators (must precede their single-char prefixes
    // at the logos level via longer-match priority) ---
    #[token("==")] EqEq,
    #[token("!=")] NotEq,
    #[token("<=")] LessEq,
    #[token(">=")] GreaterEq,
    #[token("&&")] AndAnd,
    #[token("||")] OrOr,
    #[token("<<")] ShlShl,
    #[token(">>")] ShrShr,
    #[token("=>")] FatArrow,
    #[token("->")] Arrow,

    // --- single-char operators & punctuation ---
    #[token("+")] Plus,
    #[token("-")] Minus,
    #[token("*")] Star,
    #[token("/")] Slash,
    #[token("%")] Percent,
    #[token("=")] Assign,
    #[token("&")] Amp,
    #[token("|")] Pipe,
    #[token("^")] Caret,
    #[token(">")] Gt,
    #[token("<")] Lt,
    #[token("!")] Bang,
    #[token("(")] LParen,
    #[token(")")] RParen,
    #[token("{")] LBrace,
    #[token("}")] RBrace,
    #[token("[")] LBracket,
    #[token("]")] RBracket,
    #[token(",")] Comma,
    #[token(";")] Semi,
    #[token(":")] Colon,
    #[token(".")] Dot,
    #[token("?")] Question,

    // --- block comments: skip via callback ---
    #[regex(r"/\*", block_comment)]
    BlockComment,

    // --- literals ---
    // Hex / binary int literals (with `_` digit separators). Priority
    // is bumped so `0x10` never decomposes into `Int(0) Ident("x10")`.
    #[regex(r"0[xX][0-9a-fA-F_]+", hex_int, priority = 3)]
    HexInt(i64),
    #[regex(r"0[bB][01_]+", bin_int, priority = 3)]
    BinInt(i64),
    // Float literal — `1.2`, `1.`. Must outrank `Int` for inputs with
    // a trailing `.` so logos picks the float arm.
    #[regex(r"[0-9]+\.[0-9]*", |lex| lex.slice().parse::<f64>().ok(), priority = 2)]
    Float(f64),
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().ok())]
    Int(i64),
    // String literal — `"([^"\\]|\\[\s\S])*"` matches any non-quote
    // non-backslash char OR a backslash followed by any char
    // (including newlines). `string_lit` post-processes escapes.
    #[regex(r#""([^"\\]|\\[\s\S])*""#, string_lit)]
    Str(String),

    // --- keywords ---
    #[token("fn")] Fn,
    #[token("let")] Let,
    #[token("live")] Live,
    #[token("assert")] Assert,
    #[token("if")] If,
    #[token("else")] Else,
    #[token("return")] Return,
    #[token("static")] Static,
    #[token("while")] While,
    #[token("for")] For,
    #[token("in")] In,
    #[token("requires")] Requires,
    #[token("ensures")] Ensures,
    #[token("invariant")] Invariant,
    #[token("struct")] Struct,
    #[token("new")] New,
    #[token("match")] Match,
    #[token("use")] Use,
    #[token("true")] True,
    #[token("false")] False,
    #[token("_")] Underscore,

    // --- identifiers ---
    // Split into two arms so bare `_` is handled by the `#[token]`
    // above and everything else (`foo`, `_foo`, `x1`) lands here.
    #[regex(r"[a-zA-Z][a-zA-Z0-9_]*|_[a-zA-Z0-9_]+", |lex| lex.slice().to_string())]
    Ident(String),
}

fn hex_int(lex: &mut logos::Lexer<Tok>) -> Option<i64> {
    let body = lex.slice()[2..].replace('_', "");
    // Matches the hand-rolled lexer's best-effort fallback: on overflow
    // or an empty body (which the regex's `+` already forbids, but we
    // keep the guard to mirror semantics), emit 0.
    Some(i64::from_str_radix(&body, 16).unwrap_or(0))
}

fn bin_int(lex: &mut logos::Lexer<Tok>) -> Option<i64> {
    let body = lex.slice()[2..].replace('_', "");
    Some(i64::from_str_radix(&body, 2).unwrap_or(0))
}

fn string_lit(lex: &mut logos::Lexer<Tok>) -> String {
    // The matched slice includes surrounding quotes; strip them.
    let slice = lex.slice();
    let inner = &slice[1..slice.len().saturating_sub(1)];
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    // Unknown escape: preserve as backslash + char,
                    // matching the hand-rolled `read_string`.
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn block_comment(lex: &mut logos::Lexer<Tok>) -> logos::Skip {
    // `lex.slice()` already covered the opening `/*`; scan the
    // remainder for the first `*/` and bump past it. On EOF without
    // a closer, consume to end of input so the lexer stops.
    let rem = lex.remainder();
    match rem.find("*/") {
        Some(end) => lex.bump(end + 2),
        None => lex.bump(rem.len()),
    }
    logos::Skip
}

/// Tokenize `src` through the logos-derived scanner and return a
/// `(Token, Span)` stream terminated by `Token::Eof`.
///
/// The returned spans are in the crate's `Span` format — 1-indexed
/// line and column, 0-indexed char offset — so downstream diagnostics
/// receive the same shape they do from the hand-rolled lexer.
///
/// Implementation: build the line-start table once with
/// `Lexer::build_line_table` (RES-110), then backfill every logos
/// byte span via `pos_from_byte`. Binary search keeps the conversion
/// O(log n) per token.
pub fn tokenize(src: &str) -> Vec<(Token, Span)> {
    let table = crate::Lexer::build_line_table(src);

    let mut out: Vec<(Token, Span)> = Vec::new();
    let mut lex = Tok::lexer(src);
    while let Some(result) = lex.next() {
        let range = lex.span();
        let start = pos_from_byte(&table, src, range.start);
        let end = pos_from_byte(&table, src, range.end);
        let span = Span::new(start, end);
        let tok = match result {
            Ok(t) => convert(t),
            Err(_) => {
                // A char that matched no rule — legacy lexer emits
                // `Token::Unknown(ch)` with the offending char.
                let first_char = lex.slice().chars().next().unwrap_or('\0');
                Token::Unknown(first_char)
            }
        };
        out.push((tok, span));
    }
    // Terminate the stream with a sentinel Eof at the final position,
    // mirroring what the hand-rolled lexer produces after input ends.
    //
    // The hand-rolled lexer calls `read_char` once more after emitting
    // `Token::Eof`, which bumps its `column` *and* char-offset by one
    // even though no character was actually consumed. We reproduce
    // that single-unit bump on the end position so the parity test
    // matches; downstream consumers only look at the `start` of an
    // EOF span anyway.
    let eof_pos = pos_from_byte(&table, src, src.len());
    let eof_end = Pos::new(eof_pos.line, eof_pos.column + 1, eof_pos.offset + 1);
    out.push((Token::Eof, Span::new(eof_pos, eof_end)));
    out
}

fn convert(t: Tok) -> Token {
    match t {
        Tok::Fn => Token::Function,
        Tok::Let => Token::Let,
        Tok::Live => Token::Live,
        Tok::Assert => Token::Assert,
        Tok::If => Token::If,
        Tok::Else => Token::Else,
        Tok::Return => Token::Return,
        Tok::Static => Token::Static,
        Tok::While => Token::While,
        Tok::For => Token::For,
        Tok::In => Token::In,
        Tok::Requires => Token::Requires,
        Tok::Ensures => Token::Ensures,
        Tok::Invariant => Token::Invariant,
        Tok::Struct => Token::Struct,
        Tok::New => Token::New,
        Tok::Match => Token::Match,
        Tok::Use => Token::Use,
        Tok::True => Token::BoolLiteral(true),
        Tok::False => Token::BoolLiteral(false),
        Tok::Underscore => Token::Underscore,
        Tok::EqEq => Token::Equal,
        Tok::NotEq => Token::NotEqual,
        Tok::LessEq => Token::LessEqual,
        Tok::GreaterEq => Token::GreaterEqual,
        Tok::AndAnd => Token::And,
        Tok::OrOr => Token::Or,
        Tok::ShlShl => Token::ShiftLeft,
        Tok::ShrShr => Token::ShiftRight,
        Tok::FatArrow => Token::FatArrow,
        Tok::Arrow => Token::Arrow,
        Tok::Plus => Token::Plus,
        Tok::Minus => Token::Minus,
        Tok::Star => Token::Multiply,
        Tok::Slash => Token::Divide,
        Tok::Percent => Token::Modulo,
        Tok::Assign => Token::Assign,
        Tok::Amp => Token::BitAnd,
        Tok::Pipe => Token::BitOr,
        Tok::Caret => Token::BitXor,
        Tok::Gt => Token::Greater,
        Tok::Lt => Token::Less,
        Tok::Bang => Token::Bang,
        Tok::LParen => Token::LeftParen,
        Tok::RParen => Token::RightParen,
        Tok::LBrace => Token::LeftBrace,
        Tok::RBrace => Token::RightBrace,
        Tok::LBracket => Token::LeftBracket,
        Tok::RBracket => Token::RightBracket,
        Tok::Comma => Token::Comma,
        Tok::Semi => Token::Semicolon,
        Tok::Colon => Token::Colon,
        Tok::Dot => Token::Dot,
        Tok::Question => Token::Question,
        Tok::HexInt(n) => Token::IntLiteral(n),
        Tok::BinInt(n) => Token::IntLiteral(n),
        Tok::Int(n) => Token::IntLiteral(n),
        Tok::Float(f) => Token::FloatLiteral(f),
        Tok::Str(s) => Token::StringLiteral(s),
        Tok::Ident(s) => Token::Identifier(s),
        // `BlockComment` is never emitted — its callback always
        // returns `logos::Skip`. Presence here guards the match's
        // exhaustiveness without special-casing it above.
        Tok::BlockComment => unreachable!("block_comment callback skips its variant"),
    }
}
