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
    // RES-149: set-literal opener `#{`. Must precede any lone-`#`
    // handling so logos prefers the longer match.
    #[token("#{")] HashLBrace,

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
    // RES-191: attribute prefix (`@pure`, etc.). Emitted as a bare
    // `@`; the parser reads the following identifier.
    #[token("@")] At,

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
    // RES-152: byte-string literal `b"..."`. Same inner-char regex
    // as `Str` with a mandatory `b` prefix. `bytes_lit` post-
    // processes escapes (named + `\xNN`) into a `Vec<u8>`. Priority
    // is bumped above `Ident` so a bare `b` followed immediately by
    // `"..."` never decomposes into `Ident("b") Str(...)`.
    #[regex(r#"b"([^"\\]|\\[\s\S])*""#, bytes_lit, priority = 3)]
    BytesLit(Vec<u8>),

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
    // RES-158: `impl <Struct> { ... }` keyword. Added alongside the
    // hand-rolled lexer's `"impl" => Token::Impl` kw arm so feature
    // parity is preserved.
    #[token("impl")] Impl,
    // RES-128: `type <Name> = <Target>;` alias keyword. Same parity
    // requirement as above.
    #[token("type")] Type,
    #[token("true")] True,
    #[token("false")] False,
    #[token("_")] Underscore,
    // RES-163: `default` is a reserved alias for `_` at the top of
    // a match arm. Must precede the `Ident` regex so logos picks
    // the keyword arm over the identifier arm.
    #[token("default")] Default,

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

fn bytes_lit(lex: &mut logos::Lexer<Tok>) -> Vec<u8> {
    // The matched slice is `b"..."`; strip the `b"` prefix and the
    // trailing `"`.
    let slice = lex.slice();
    let inner = &slice[2..slice.len().saturating_sub(1)];
    let mut out: Vec<u8> = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push(b'\n'),
                Some('t') => out.push(b'\t'),
                Some('r') => out.push(b'\r'),
                Some('0') => out.push(0),
                Some('\\') => out.push(b'\\'),
                Some('"') => out.push(b'"'),
                Some('x') => {
                    // `\xNN` — exactly two hex digits.
                    let hi = chars.next();
                    let lo = chars.next();
                    let nibble = |c: Option<char>| -> Option<u8> {
                        match c {
                            Some('0'..='9') => Some(c.unwrap() as u8 - b'0'),
                            Some('a'..='f') => Some(c.unwrap() as u8 - b'a' + 10),
                            Some('A'..='F') => Some(c.unwrap() as u8 - b'A' + 10),
                            _ => None,
                        }
                    };
                    match (nibble(hi), nibble(lo)) {
                        (Some(h), Some(l)) => out.push((h << 4) | l),
                        _ => {
                            out.extend_from_slice(b"\\x");
                            if let Some(c) = hi
                                && c.is_ascii()
                            {
                                out.push(c as u8);
                            }
                            if let Some(c) = lo
                                && c.is_ascii()
                            {
                                out.push(c as u8);
                            }
                        }
                    }
                }
                Some(other) => {
                    // Unknown escape (including `\u{...}`) — pass
                    // through as literal `\` + following char.
                    out.push(b'\\');
                    if other.is_ascii() {
                        out.push(other as u8);
                    }
                }
                None => out.push(b'\\'),
            }
        } else if c.is_ascii() {
            out.push(c as u8);
        } else {
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            out.extend_from_slice(s.as_bytes());
        }
    }
    out
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
/// `Lexer::build_line_table` (RES-110), plus a parallel table of
/// per-line cumulative char counts so converting a byte offset to
/// a `Pos` is O(log n) for the line search + O(line-length) for
/// the column / char-offset counting. Without the char-count table
/// RES-110's `pos_from_byte` is O(byte) per call — over N tokens
/// that's O(N²) (RES-109's benchmark was crushed by this;
/// `fast_pos` below fixes it).
pub fn tokenize(src: &str) -> Vec<(Token, Span)> {
    // RES-113: honour a leading shebang line. Logos doesn't have a
    // "start of input" anchor, so we just skip the `#!..\n` prefix
    // manually and feed logos the suffix. Line/col/offset tables
    // are still built from the FULL source so the first real
    // token's span reports its true byte offset.
    let shebang_bytes: usize = if src.starts_with("#!") {
        match src.find('\n') {
            Some(nl) => nl + 1,
            None => src.len(),
        }
    } else {
        0
    };

    let table = crate::span::build_line_table(src);

    // Char count at the start of each line: entry `i` = total chars
    // in the prefix `src[..table[i]]`. Single O(n) pass over bytes;
    // each byte-start-of-line is advanced as we go. Paired with
    // `table`, the two vectors answer "char count before this line"
    // in O(1).
    let mut char_at_line_start: Vec<usize> = Vec::with_capacity(table.len());
    char_at_line_start.push(0);
    let mut cur_line = 1usize; // next table index to fill (table[0] = 0 already accounted)
    let mut cur_chars = 0usize;
    for (byte_off, _ch) in src.char_indices() {
        while cur_line < table.len() && byte_off >= table[cur_line] {
            char_at_line_start.push(cur_chars);
            cur_line += 1;
        }
        cur_chars += 1;
    }
    while char_at_line_start.len() < table.len() {
        char_at_line_start.push(cur_chars);
    }

    // O(log n) byte->Pos using both tables. Column and offset
    // measured in characters.
    let fast_pos = |byte: usize| -> Pos {
        let byte = byte.min(src.len());
        let line_idx = match table.binary_search(&byte) {
            Ok(i) => i,
            Err(0) => 0,
            Err(i) => i - 1,
        };
        let line_start = table[line_idx];
        let line = line_idx + 1;
        let col_slice = src.get(line_start..byte).unwrap_or("");
        let col_chars = col_slice.chars().count();
        let column = col_chars + 1;
        let offset = char_at_line_start[line_idx] + col_chars;
        Pos::new(line, column, offset)
    };

    let mut out: Vec<(Token, Span)> = Vec::new();
    // Feed only the post-shebang slice to logos, and offset every
    // reported byte range by `shebang_bytes` when converting to a
    // `Pos` against the full-source table.
    let mut lex = Tok::lexer(&src[shebang_bytes..]);
    while let Some(result) = lex.next() {
        let range = lex.span();
        let start = fast_pos(range.start + shebang_bytes);
        let end = fast_pos(range.end + shebang_bytes);
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
    let eof_pos = fast_pos(src.len());
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
        Tok::Impl => Token::Impl,
        Tok::Type => Token::Type,
        Tok::True => Token::BoolLiteral(true),
        Tok::False => Token::BoolLiteral(false),
        Tok::Underscore => Token::Underscore,
        Tok::Default => Token::Default,
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
        Tok::HashLBrace => Token::HashLeftBrace,
        Tok::Comma => Token::Comma,
        Tok::Semi => Token::Semicolon,
        Tok::Colon => Token::Colon,
        Tok::Dot => Token::Dot,
        Tok::Question => Token::Question,
        Tok::At => Token::At,
        Tok::HexInt(n) => Token::IntLiteral(n),
        Tok::BinInt(n) => Token::IntLiteral(n),
        Tok::Int(n) => Token::IntLiteral(n),
        Tok::Float(f) => Token::FloatLiteral(f),
        Tok::Str(s) => Token::StringLiteral(s),
        Tok::BytesLit(b) => Token::BytesLiteral(b),
        Tok::Ident(s) => Token::Identifier(s),
        // `BlockComment` is never emitted — its callback always
        // returns `logos::Skip`. Presence here guards the match's
        // exhaustiveness without special-casing it above.
        Tok::BlockComment => unreachable!("block_comment callback skips its variant"),
    }
}
