use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

// Import modules
mod typechecker;
mod repl;
mod span;
mod imports;
mod bytecode;
mod compiler;
mod disasm;
mod peephole;
mod vm;
#[cfg(feature = "z3")]
mod verifier_z3;
#[cfg(feature = "lsp")]
mod lsp_server;
#[cfg(feature = "jit")]
mod jit_backend;
// RES-108: opt-in logos-based lexer. See module docs; the feature
// flag gates the routing so the legacy hand-rolled scanner stays
// authoritative until RES-109 benchmarks land.
#[cfg(feature = "logos-lexer")]
mod lexer_logos;
// RES-121: Hindley-Milner unification + occurs check. Unconditionally
// compiled; consumed by the inference walker when RES-120 lands.
mod unify;
// RES-117: shared diagnostic rendering (caret underlines under
// the offending source span). Used by the driver when formatting
// parser / typechecker / interpreter / VM errors.
mod diag;
// RES-205: `resilient pkg init <name>` — project scaffolding.
// Standalone from the compiler pipeline; lives here so the single
// `resilient` binary carries it alongside the runtime.
mod pkg_init;

#[allow(unused_imports)]
use span::{Pos, Span, Spanned};

use rustyline::error::ReadlineError;
use rustyline::{DefaultEditor, Result as RustylineResult};

// Token types for our lexer
#[derive(Debug, Clone, PartialEq)]
enum Token {
    // Keywords
    Function,
    Let,
    Live,
    Assert,
    If,
    Else,
    Return,
    Static,
    While,
    For,
    In,
    Requires,
    Ensures,
    Invariant,
    Struct,
    New,
    Dot,
    Match,
    FatArrow,
    Arrow,
    Underscore,
    /// RES-163: `default` — alias for `_` at the top of a match
    /// arm. Reserved word: `default` cannot appear as an
    /// identifier, so `let default = 3;` is a parse error. The
    /// parser desugars the match-arm use to `Pattern::Wildcard`
    /// so downstream phases never see a distinct "default"
    /// pattern.
    Default,
    Question,
    /// RES-073: `use "path/to/file.res";` — module import.
    Use,
    /// RES-158: `impl <StructName> { fn method(self, ...) { ... } }`.
    Impl,
    /// RES-128: `type <Name> = <Target>;` non-nominal alias.
    Type,
    
    // Literals
    Identifier(String),
    IntLiteral(i64),
    FloatLiteral(f64),
    StringLiteral(String),
    BoolLiteral(bool),
    /// RES-152: `b"..."` byte-string literal. The lexer decodes the
    /// contents into raw `Vec<u8>` at tokenization time — hex
    /// escapes (`\xNN`), the five named escapes (`\n`, `\t`, `\r`,
    /// `\0`, `\\`, `\"`), and any ASCII-printable bytes. Unknown
    /// escapes (including `\u{...}`, which is deliberately NOT a
    /// Unicode code-point at the bytes level) pass through as the
    /// literal two bytes `\` + char — see ticket Notes.
    BytesLiteral(Vec<u8>),
    
    // Operators
    Plus,
    Minus,
    Multiply,
    Divide,
    Modulo,
    Assign,
    Equal,
    NotEqual,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
    Greater,
    Less,
    GreaterEqual,
    LessEqual,
    
    // Delimiters
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    /// RES-149: opener for a set literal `#{...}`. Emitted as a
    /// single token so the parser can disambiguate from the bare
    /// `{` (map / block) without look-ahead. The closing `}` is
    /// an ordinary `RightBrace`.
    HashLeftBrace,
    Comma,
    Semicolon,
    Colon,
    
    /// Prefix logical-not.
    Bang,
    /// RES-191: attribute prefix — `@pure`, `@inline`, etc. Only
    /// `@pure` is recognized today; unknown annotations are a
    /// parse error. Carried as a bare `At` token so the parser
    /// can read the following identifier and decide what to do.
    At,

    // Other
    Eof,
    /// A character the lexer did not recognize. Emitted instead of
    /// panicking so the parser can report a graceful diagnostic. The
    /// `char` payload is the offending character, for the error message.
    Unknown(char),
}

impl std::fmt::Display for Token {
    /// RES-118: `{}` on a `Token` produces the user-facing syntax
    /// form (`;` rather than `Semicolon`, `identifier `name`` rather
    /// than `Identifier("name")`). Parser error sites that used
    /// `{:?}` for the "found X" suffix can just switch to `{}` and
    /// pick up readable messages without touching arguments.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display_syntax())
    }
}

impl Token {
    /// RES-118: user-facing syntax name for this token, used in
    /// parser diagnostics. Intentionally echoes the source form
    /// users actually type (`;` not `Semicolon`, `{` not
    /// `LeftBrace`) so error messages read naturally. Payload-
    /// carrying variants render the payload inline when it's
    /// informative (`<ident>`, a string / int / float literal's
    /// written form) or a `<category>` placeholder when the
    /// specific value is irrelevant to the diagnostic.
    pub fn display_syntax(&self) -> String {
        match self {
            Token::Function => "`fn`".to_string(),
            Token::Let => "`let`".to_string(),
            Token::Live => "`live`".to_string(),
            Token::Assert => "`assert`".to_string(),
            Token::If => "`if`".to_string(),
            Token::Else => "`else`".to_string(),
            Token::Return => "`return`".to_string(),
            Token::Static => "`static`".to_string(),
            Token::While => "`while`".to_string(),
            Token::For => "`for`".to_string(),
            Token::In => "`in`".to_string(),
            Token::Requires => "`requires`".to_string(),
            Token::Ensures => "`ensures`".to_string(),
            Token::Invariant => "`invariant`".to_string(),
            Token::Struct => "`struct`".to_string(),
            Token::New => "`new`".to_string(),
            Token::Match => "`match`".to_string(),
            Token::Use => "`use`".to_string(),
            Token::Impl => "`impl`".to_string(),
            Token::Type => "`type`".to_string(),
            Token::Underscore => "`_`".to_string(),
            Token::Default => "`default`".to_string(),
            Token::Dot => "`.`".to_string(),
            Token::FatArrow => "`=>`".to_string(),
            Token::Arrow => "`->`".to_string(),
            Token::Question => "`?`".to_string(),
            Token::Plus => "`+`".to_string(),
            Token::Minus => "`-`".to_string(),
            Token::Multiply => "`*`".to_string(),
            Token::Divide => "`/`".to_string(),
            Token::Modulo => "`%`".to_string(),
            Token::Assign => "`=`".to_string(),
            Token::Equal => "`==`".to_string(),
            Token::NotEqual => "`!=`".to_string(),
            Token::And => "`&&`".to_string(),
            Token::Or => "`||`".to_string(),
            Token::BitAnd => "`&`".to_string(),
            Token::BitOr => "`|`".to_string(),
            Token::BitXor => "`^`".to_string(),
            Token::ShiftLeft => "`<<`".to_string(),
            Token::ShiftRight => "`>>`".to_string(),
            Token::Greater => "`>`".to_string(),
            Token::Less => "`<`".to_string(),
            Token::GreaterEqual => "`>=`".to_string(),
            Token::LessEqual => "`<=`".to_string(),
            Token::LeftParen => "`(`".to_string(),
            Token::RightParen => "`)`".to_string(),
            Token::LeftBrace => "`{`".to_string(),
            Token::RightBrace => "`}`".to_string(),
            Token::LeftBracket => "`[`".to_string(),
            Token::RightBracket => "`]`".to_string(),
            Token::HashLeftBrace => "`#{`".to_string(),
            Token::Comma => "`,`".to_string(),
            Token::Semicolon => "`;`".to_string(),
            Token::Colon => "`:`".to_string(),
            Token::Bang => "`!`".to_string(),
            Token::At => "`@`".to_string(),
            Token::Identifier(name) => format!("identifier `{}`", name),
            Token::IntLiteral(v) => format!("integer literal `{}`", v),
            Token::FloatLiteral(v) => format!("float literal `{}`", v),
            Token::StringLiteral(_) => "string literal".to_string(),
            Token::BytesLiteral(_) => "bytes literal".to_string(),
            Token::BoolLiteral(b) => format!("`{}`", b),
            Token::Eof => "end of input".to_string(),
            Token::Unknown(c) => format!("unrecognized character `{}`", c),
        }
    }
}

/// RES-118: format an `expected one of <…>, got <token>` diagnostic
/// for parser error sites. `expected` is a slice of already-
/// user-facing syntax strings (backtick-quoted punctuation, prose
/// categories like `identifier`). Single-element slices specialize
/// to `expected X, got Y` to keep the common case reading
/// naturally. Slices longer than 5 entries are truncated with a
/// trailing `…` so deep FIRST-sets don't balloon the diagnostic.
pub fn format_expected(expected: &[&str], got_syntax: &str) -> String {
    const DISPLAY_CAP: usize = 5;
    if expected.is_empty() {
        return format!("unexpected {}", got_syntax);
    }
    if expected.len() == 1 {
        return format!("expected {}, got {}", expected[0], got_syntax);
    }
    let shown = if expected.len() > DISPLAY_CAP {
        let mut s: Vec<&str> = expected[..DISPLAY_CAP].to_vec();
        s.push("…");
        s
    } else {
        expected.to_vec()
    };
    format!("expected one of {}, got {}", shown.join(", "), got_syntax)
}

// Lexer for tokenizing Resilient source code
struct Lexer {
    input: Vec<char>,
    position: usize,
    read_position: usize,
    ch: char,
    /// 1-indexed current line, advanced each time we consume a '\n'.
    line: usize,
    /// 1-indexed column at the current `ch`.
    column: usize,
    /// Line/column snapshotted at the START of the most recently
    /// emitted token — so external code can ask "where did this
    /// token begin?".
    last_token_line: usize,
    last_token_column: usize,
    /// RES-110: char-offset snapshot taken alongside
    /// `last_token_line/column` so `next_token_with_span` can emit a
    /// `Pos` with a real `offset` (not a 0 placeholder). Indexed into
    /// `input` as a char-count, same semantics as `position`.
    last_token_offset: usize,
    /// RES-108: when the `logos-lexer` feature is enabled, `Lexer::new`
    /// pre-scans the full input via the logos-derived scanner into a
    /// cached token stream. Each `next_token` call pops the next
    /// `(Token, Span)` from here instead of driving the hand-rolled
    /// state machine above. When `None` (or when the feature is off),
    /// the legacy path is used.
    #[cfg(feature = "logos-lexer")]
    logos_tokens: Option<std::vec::IntoIter<(Token, span::Span)>>,
}

impl Lexer {
    fn new(input: String) -> Self {
        #[cfg(feature = "logos-lexer")]
        {
            // RES-108: under the `logos-lexer` feature, pre-tokenize
            // the entire input through the logos-based scanner.
            // Subsequent `next_token` calls drain the cached stream;
            // the legacy scan state (`input`, `position`, etc.) stays
            // initialized so diagnostics that reach into fields like
            // `last_token_line` still work.
            let tokens = lexer_logos::tokenize(&input);
            Lexer {
                input: input.chars().collect(),
                position: 0,
                read_position: 0,
                ch: '\0',
                line: 1,
                column: 0,
                last_token_line: 1,
                last_token_column: 1,
                last_token_offset: 0,
                logos_tokens: Some(tokens.into_iter()),
            }
        }
        #[cfg(not(feature = "logos-lexer"))]
        {
            let mut lexer = Lexer {
                input: input.chars().collect(),
                position: 0,
                read_position: 0,
                ch: '\0',
                line: 1,
                column: 0,
                last_token_line: 1,
                last_token_column: 1,
                last_token_offset: 0,
            };
            lexer.read_char();
            // RES-113: silently consume a leading shebang line
            // (`#!...\n`) at byte 0 of the input so users can make
            // Resilient scripts executable with `#!/usr/bin/env
            // resilient`. `read_char` naturally advances
            // `line / column / position`, so the first real token's
            // span points at its true byte offset — we don't
            // subtract the shebang length.
            if lexer.ch == '#' && lexer.peek_char() == '!' {
                while lexer.ch != '\n' && lexer.ch != '\0' {
                    lexer.read_char();
                }
                if lexer.ch == '\n' {
                    lexer.read_char();
                }
            }
            lexer
        }
    }

    fn read_char(&mut self) {
        if self.ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        if self.read_position >= self.input.len() {
            self.ch = '\0';
        } else {
            self.ch = self.input[self.read_position];
        }
        self.position = self.read_position;
        self.read_position += 1;
    }
    
    fn peek_char(&self) -> char {
        if self.read_position >= self.input.len() {
            '\0'
        } else {
            self.input[self.read_position]
        }
    }
    
    fn next_token(&mut self) -> Token {
        // RES-108: under the `logos-lexer` feature, drain the pre-
        // scanned stream. Each pop also updates the legacy line/col
        // trackers so downstream code that inspects `last_token_line`
        // / `last_token_column` (e.g. the parser's error-position
        // helpers) keeps working transparently.
        #[cfg(feature = "logos-lexer")]
        if let Some(iter) = self.logos_tokens.as_mut() {
            if let Some((tok, span)) = iter.next() {
                self.last_token_line = span.start.line;
                self.last_token_column = span.start.column;
                self.last_token_offset = span.start.offset;
                self.line = span.end.line;
                self.column = span.end.column;
                self.position = span.end.offset;
                return tok;
            }
            return Token::Eof;
        }
        self.skip_whitespace();
        // Capture where this token STARTS so `Parser` can attribute
        // errors to the correct file:line:col.
        self.last_token_line = self.line;
        self.last_token_column = self.column;
        self.last_token_offset = self.position;
        
        let token = match self.ch {
            '=' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::Equal
                } else if self.peek_char() == '>' {
                    self.read_char();
                    Token::FatArrow
                } else {
                    Token::Assign
                }
            },
            '+' => Token::Plus,
            '-' => {
                if self.peek_char() == '>' {
                    self.read_char();
                    Token::Arrow
                } else {
                    Token::Minus
                }
            },
            '*' => Token::Multiply,
            '%' => Token::Modulo,
            '&' => {
                if self.peek_char() == '&' {
                    self.read_char();
                    Token::And
                } else {
                    Token::BitAnd
                }
            },
            '|' => {
                if self.peek_char() == '|' {
                    self.read_char();
                    Token::Or
                } else {
                    Token::BitOr
                }
            },
            '^' => Token::BitXor,
            '/' => {
                if self.peek_char() == '/' {
                    // Line comment: skip to newline.
                    while self.ch != '\n' && self.ch != '\0' {
                        self.read_char();
                    }
                    return self.next_token();
                } else if self.peek_char() == '*' {
                    // Block comment: skip to '*/' (non-nesting).
                    self.read_char(); // consume first '*'
                    self.read_char(); // advance past it
                    loop {
                        if self.ch == '\0' {
                            // Unterminated block comment — record a lexer
                            // error and stop; return Eof so parser stops.
                            return Token::Unknown('*');
                        }
                        if self.ch == '*' && self.peek_char() == '/' {
                            self.read_char(); // '*'
                            self.read_char(); // '/'
                            break;
                        }
                        self.read_char();
                    }
                    return self.next_token();
                } else {
                    Token::Divide
                }
            },
            '>' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::GreaterEqual
                } else if self.peek_char() == '>' {
                    self.read_char();
                    Token::ShiftRight
                } else {
                    Token::Greater
                }
            },
            '<' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::LessEqual
                } else if self.peek_char() == '<' {
                    self.read_char();
                    Token::ShiftLeft
                } else {
                    Token::Less
                }
            },
            '!' => {
                if self.peek_char() == '=' {
                    self.read_char();
                    Token::NotEqual
                } else {
                    Token::Bang
                }
            },
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            '{' => Token::LeftBrace,
            '}' => Token::RightBrace,
            '[' => Token::LeftBracket,
            ']' => Token::RightBracket,
            // RES-149: set literal opener `#{...}`. A lone `#` (or
            // `#` followed by anything other than `{`) still falls
            // through to `Token::Unknown('#')` below — shebangs are
            // consumed at file head before we ever reach here.
            '#' if self.peek_char() == '{' => {
                self.read_char(); // consume `{` so the outer
                // `self.read_char()` at the end of `next_token`
                // advances past it naturally.
                Token::HashLeftBrace
            },
            // RES-038: `.` is now a real token (field access). Numeric
            // literals are still fine because read_number consumes `.`
            // before the tokenizer can dispatch here — digit check
            // comes first in next_token's fall-through arm.
            '.' => Token::Dot,
            '?' => Token::Question,
            ',' => Token::Comma,
            ';' => Token::Semicolon,
            ':' => Token::Colon,
            // RES-191: attribute prefix (e.g. `@pure`). The parser
            // consumes the following identifier; the lexer just
            // tags the `@` itself.
            '@' => Token::At,
            '"' => {
                self.read_char();
                let str_value = self.read_string();
                Token::StringLiteral(str_value)
            },
            // RES-152: `b"..."` byte-string literal. The guard on
            // the next char distinguishes from a bare identifier
            // that starts with `b` — ASCII letters still fall
            // through to `read_identifier` below.
            'b' if self.peek_char() == '"' => {
                self.read_char(); // consume `b`; self.ch == '"'
                self.read_char(); // consume `"`; self.ch is first content byte or closing `"`
                let bytes = self.read_bytes();
                Token::BytesLiteral(bytes)
            },
            '\0' => Token::Eof,
            _ => {
                if self.is_letter(self.ch) {
                    // read_identifier() leaves self.ch at the first character
                    // AFTER the identifier, so we early-return without the
                    // trailing read_char() to avoid swallowing it.
                    let ident = self.read_identifier();
                    return match ident.as_str() {
                        "fn" => Token::Function,
                        "let" => Token::Let,
                        "live" => Token::Live,
                        "assert" => Token::Assert,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "return" => Token::Return,
                        "static" => Token::Static,
                        "while" => Token::While,
                        "for" => Token::For,
                        "in" => Token::In,
                        "requires" => Token::Requires,
                        "ensures" => Token::Ensures,
                        "invariant" => Token::Invariant,
                        "struct" => Token::Struct,
                        "new" => Token::New,
                        "match" => Token::Match,
                        "use" => Token::Use,
                        "impl" => Token::Impl,
                        "type" => Token::Type,
                        "_" => Token::Underscore,
                        // RES-163: `default` is a reserved alias
                        // for `_` at the top of a match arm.
                        // Outside that position the parser rejects
                        // it as an unexpected token — which is the
                        // "`default` as an identifier is a lex
                        // error" rule the ticket calls for.
                        "default" => Token::Default,
                        "true" => Token::BoolLiteral(true),
                        "false" => Token::BoolLiteral(false),
                        _ => Token::Identifier(ident),
                    };
                } else if self.is_digit(self.ch) {
                    return self.read_number();
                } else {
                    // Unknown character: emit a token the parser can
                    // route through `record_error` and keep going.
                    let unknown = self.ch;
                    self.read_char();
                    return Token::Unknown(unknown);
                }
            }
        };

        self.read_char();
        token
    }

    /// RES-069 (G6 partial): emit a token plus the source span it
    /// covered. The start position is the snapshot taken at the head
    /// of `next_token`; the end position reflects the lexer's cursor
    /// AFTER the token was consumed. Both are 1-indexed for line and
    /// column; offset is 0-indexed into the input char buffer.
    ///
    /// Existing call sites still use `next_token()` and ignore spans —
    /// they will migrate as the AST gains span fields.
    #[allow(dead_code)]
    fn next_token_with_span(&mut self) -> (Token, span::Span) {
        // `next_token` snapshots line / column / char-offset at the
        // first non-whitespace character of the token into
        // `last_token_*`, then advances the cursor. We use those for
        // the start position, and read `line / column / position`
        // directly for the end position.
        //
        // RES-110 promoted `start.offset` from a hardcoded 0 to the
        // snapshot in `last_token_offset`; the parity test in `mod
        // tests` now compares offsets too.
        let token = self.next_token();
        let start = span::Pos::new(
            self.last_token_line,
            self.last_token_column,
            self.last_token_offset,
        );
        let end = span::Pos::new(self.line, self.column, self.position);
        (token, span::Span::new(start, end))
    }

    fn read_identifier(&mut self) -> String {
        let position = self.position;
        while self.is_letter(self.ch) || self.is_digit(self.ch) {
            self.read_char();
        }
        self.input[position..self.position].iter().collect()
    }
    
    fn read_number(&mut self) -> Token {
        // Hex (0x...) and binary (0b...) integer literals first.
        if self.ch == '0' && (self.peek_char() == 'x' || self.peek_char() == 'X') {
            return self.read_radix_number(16, "0x");
        }
        if self.ch == '0' && (self.peek_char() == 'b' || self.peek_char() == 'B') {
            return self.read_radix_number(2, "0b");
        }

        let position = self.position;
        let mut is_float = false;

        while self.is_digit(self.ch) || self.ch == '.' {
            if self.ch == '.' {
                is_float = true;
            }
            self.read_char();
        }

        let number_str: String = self.input[position..self.position].iter().collect();

        if is_float {
            Token::FloatLiteral(number_str.parse::<f64>().unwrap_or(0.0))
        } else {
            Token::IntLiteral(number_str.parse::<i64>().unwrap_or(0))
        }
    }

    /// Consume a `0xHH..` or `0bBB..` integer literal. `prefix` is the
    /// two-character start marker already verified by the caller.
    fn read_radix_number(&mut self, radix: u32, prefix: &str) -> Token {
        // Skip the two-char prefix.
        self.read_char();
        self.read_char();
        let position = self.position;
        let is_valid_digit = |ch: char, r: u32| ch.is_digit(r) || ch == '_';
        while is_valid_digit(self.ch, radix) {
            self.read_char();
        }
        let raw: String = self.input[position..self.position].iter().collect();
        let cleaned = raw.replace('_', "");
        if cleaned.is_empty() {
            // Malformed literal like bare `0x` — best-effort: emit 0.
            // Parser already surfaces these via its own diagnostics if
            // they appear in unexpected positions.
            return Token::IntLiteral(0);
        }
        match i64::from_str_radix(&cleaned, radix) {
            Ok(n) => Token::IntLiteral(n),
            Err(_) => {
                // Overflow or invalid — fall back to 0 and let the
                // parser (or runtime) catch anomalies. A real language
                // would report this through the diagnostics pipeline;
                // once the lexer gains a diagnostic channel (G5), this
                // branch should use it. For now: note the prefix in
                // the returned string representation of a dummy token.
                let _ = prefix;
                Token::IntLiteral(0)
            }
        }
    }
    
    fn read_string(&mut self) -> String {
        let _position = self.position;
        let mut result = String::new();

        while self.ch != '"' && self.ch != '\0' {
            // Handle escape sequences
            if self.ch == '\\' && self.read_position < self.input.len() {
                self.read_char(); // Skip the backslash

                // Process escape sequence
                match self.ch {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '\\' => result.push('\\'),
                    '"' => result.push('"'),
                    _ => {
                        // Invalid escape sequence, treat as literal
                        result.push('\\');
                        result.push(self.ch);
                    }
                }
            } else {
                result.push(self.ch);
            }

            self.read_char();
        }

        result
    }

    /// RES-152: read the contents of a `b"..."` byte literal, leaving
    /// `self.ch` at the closing `"` so the outer `next_token` tail
    /// can consume it (mirrors `read_string`). Supported escapes:
    /// `\xNN` (two hex digits), `\n`, `\t`, `\r`, `\0`, `\\`, `\"`.
    /// Unknown escapes (including `\u{...}`) pass through as the
    /// literal two bytes `\` + the following char per the ticket's
    /// "Unicode escapes are disallowed" guidance: we simply don't
    /// interpret them — users who write `\u` get the six literal
    /// bytes, not a code point.
    fn read_bytes(&mut self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::new();
        while self.ch != '"' && self.ch != '\0' {
            if self.ch == '\\' && self.read_position < self.input.len() {
                self.read_char(); // past `\`
                match self.ch {
                    'n' => out.push(b'\n'),
                    't' => out.push(b'\t'),
                    'r' => out.push(b'\r'),
                    '0' => out.push(0),
                    '\\' => out.push(b'\\'),
                    '"' => out.push(b'"'),
                    'x' => {
                        // `\xNN` — exactly two hex digits.
                        let hi = self.peek_char();
                        // We need to peek two ahead, but the lexer
                        // only exposes a single-char peek. Advance
                        // manually so we can re-read.
                        self.read_char(); // self.ch == first hex digit
                        let lo = self.peek_char();
                        self.read_char(); // self.ch == second hex digit
                        let nibble = |c: char| -> Option<u8> {
                            match c {
                                '0'..='9' => Some(c as u8 - b'0'),
                                'a'..='f' => Some(c as u8 - b'a' + 10),
                                'A'..='F' => Some(c as u8 - b'A' + 10),
                                _ => None,
                            }
                        };
                        match (nibble(hi), nibble(lo)) {
                            (Some(h), Some(l)) => out.push((h << 4) | l),
                            _ => {
                                // Malformed — emit literal `\x` plus
                                // whatever bytes we consumed so the
                                // source is still recoverable.
                                out.extend_from_slice(b"\\x");
                                if hi.is_ascii() {
                                    out.push(hi as u8);
                                }
                                if lo.is_ascii() {
                                    out.push(lo as u8);
                                }
                            }
                        }
                    }
                    other => {
                        // Unknown escape. Pass through as `\` + the
                        // following char (best-effort), matching
                        // `read_string`'s forgiveness. Notably
                        // includes `\u{...}` — byte literals don't
                        // honor Unicode escapes per the ticket's
                        // Notes.
                        out.push(b'\\');
                        if other.is_ascii() {
                            out.push(other as u8);
                        }
                    }
                }
            } else if self.ch.is_ascii() {
                out.push(self.ch as u8);
            } else {
                // Non-ASCII char inside a byte literal: store its
                // UTF-8 encoding as-is. The ticket nudges users
                // toward `\xNN` for anything non-printable, but we
                // don't force them; emitting the UTF-8 bytes keeps
                // the lexer predictable.
                let mut buf = [0u8; 4];
                let s = self.ch.encode_utf8(&mut buf);
                out.extend_from_slice(s.as_bytes());
            }
            self.read_char();
        }
        out
    }
    
    fn is_letter(&self, ch: char) -> bool {
        // RES-114: ASCII-only identifier policy. Restrict to
        // `[A-Za-z_]` so homoglyph attacks (Cyrillic `kafa` vs
        // Latin `kafa`, Greek `Α` vs Latin `A`, etc.) can't
        // produce visually-identical but distinct identifiers.
        // String / comment bodies retain full UTF-8 — only
        // identifier scanning is tightened. The logos lexer's
        // identifier regex is already ASCII-only, so both paths
        // agree. Non-ASCII at an identifier position falls through
        // to `Token::Unknown(ch)`; the parser's record_error
        // branch on that arm surfaces the dedicated diagnostic
        // "identifier contains non-ASCII character".
        ch.is_ascii_alphabetic() || ch == '_'
    }
    
    fn is_digit(&self, ch: char) -> bool {
        ch.is_ascii_digit()
    }
    
    fn skip_whitespace(&mut self) {
        while self.ch.is_whitespace() {
            self.read_char();
        }
    }

    // RES-115: `build_line_table` / `pos_from_byte` moved to the
    // `resilient-span` crate. Callers keep using the same names via
    // the `crate::span::*` re-export shim in `span.rs`.
}

/// RES-039: patterns for `match` arms.
#[derive(Debug, Clone)]
enum Pattern {
    /// Matches a literal int, float, string, or bool.
    Literal(Node),
    /// Binds the scrutinee to an identifier; always matches.
    Identifier(String),
    /// Matches anything without binding (`_`).
    Wildcard,
    /// RES-160: `p1 | p2 | ...` — matches if any branch matches.
    /// First-match wins. All branches must bind the same set of
    /// names (checked at typecheck time) so the arm body can
    /// reliably reference bindings regardless of which branch
    /// fired.
    Or(Vec<Pattern>),
}

/// RES-139: exponential-backoff policy for a `live` block. Sleep
/// between retries on a capped exponential curve
/// `min(max_ms, base_ms * factor^retries)`. Kwargs are parsed from
/// the `live backoff(base_ms=N, factor=K, max_ms=M) { ... }` prefix;
/// any missing kwarg uses the ticket's default.
///
/// `factor` is capped at 10 — the parser rejects values above that
/// to prevent an accidental `factor=1e9` runaway that would block
/// the interpreter thread for hours on the first retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffConfig {
    pub base_ms: u64,
    pub factor: u64,
    pub max_ms: u64,
}

impl BackoffConfig {
    /// Ticket defaults: `base_ms=1`, `factor=2`, `max_ms=100`.
    pub const fn default_ticket() -> Self {
        Self { base_ms: 1, factor: 2, max_ms: 100 }
    }

    /// Sleep duration for `retries` completed (retries=0 → first
    /// retry after the first failure; schedule the body's fresh
    /// attempt `min(max_ms, base_ms * factor^retries)` ms later).
    /// Uses `saturating_pow` / `saturating_mul` so an aggressive
    /// `factor^retries` can't overflow `u64`.
    pub fn delay_ms(&self, retries: u32) -> u64 {
        let growth = (self.factor).saturating_pow(retries);
        let want = self.base_ms.saturating_mul(growth);
        want.min(self.max_ms)
    }
}

// AST nodes for our parser
#[derive(Debug, Clone)]
enum Node {
    /// RES-077 (G6 partial): top-level statements carry source spans
    /// so diagnostics can point at the originating line:col. Sub-
    /// expressions inside each statement still have no spans —
    /// RES-078 / RES-079 cover those.
    Program(Vec<span::Spanned<Node>>),
    /// RES-073: top-level `use "path";` import. The path is resolved
    /// relative to the file containing the `use`. Resolved away by
    /// `expand_uses` (in `imports.rs`) before the program reaches the
    /// typechecker or interpreter — never seen at eval time.
    Use {
        path: String,
        /// RES-088: span of the `use` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    Function {
        name: String,
        parameters: Vec<(String, String)>, // (type, name)
        body: Box<Node>,
        /// RES-035: pre-condition clauses, checked on entry. Each is a
        /// boolean expression over the parameters.
        requires: Vec<Node>,
        /// RES-035: post-condition clauses, checked on exit. The
        /// special identifier `result` is bound to the return value
        /// inside each clause's env.
        ensures: Vec<Node>,
        /// RES-052: optional `-> TYPE` return-type annotation. Advisory.
        #[allow(dead_code)]
        return_type: Option<String>,
        /// RES-088: span of the `fn` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
        /// RES-191: `@pure` annotation — the fn promises to be
        /// side-effect-free. The purity checker in
        /// `typechecker::check_purity` verifies this at type-check
        /// time; an un-annotated fn defaults to `false` and the
        /// checker ignores it entirely. A future ticket (RES-192)
        /// infers purity for unannotated fns.
        #[allow(dead_code)] // read via pattern destructure in typechecker
        pure: bool,
    },
    LiveBlock {
        body: Box<Node>,
        /// RES-036: zero or more invariant expressions checked after
        /// every iteration of the body. A failing invariant triggers
        /// the same retry path as a body-level error.
        invariants: Vec<Node>,
        /// RES-139: optional exponential-backoff policy set via the
        /// `live backoff(base_ms=..., factor=..., max_ms=...) { ... }`
        /// prefix. `None` → zero-sleep retries (the original
        /// behaviour; existing `live { ... }` stays unchanged).
        backoff: Option<BackoffConfig>,
        /// RES-142: optional wall-clock budget via the
        /// `live within <duration> { ... }` clause. `Some(dl)` carries
        /// a `Node::DurationLiteral` with the parsed nanoseconds;
        /// `None` means no cap (classic retry-forever-up-to-MAX
        /// semantics). Backoff and timeout coexist: backoff sleeps
        /// count against the budget.
        timeout: Option<Box<Node>>,
        /// RES-088: span of the `live` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-142: `<integer><unit>` duration literal, where unit ∈
    /// {`ns`, `us`, `ms`, `s`}. Deliberately narrow: the parser only
    /// emits this inside a `live ... within <duration> { ... }`
    /// clause — it's not a general-purpose expression. See the
    /// ticket's `## Notes`: "Duration literals are not a full time
    /// library — they only exist inside live clauses for now."
    DurationLiteral {
        /// Total nanoseconds the literal represents. `10ms` →
        /// `10_000_000`. Stored as `u64` to stay legal in a no_std
        /// embedded target; `Duration::from_nanos` accepts `u64`.
        nanos: u64,
        #[allow(dead_code)]
        span: span::Span,
    },
    Assert {
        condition: Box<Node>,
        message: Option<Box<Node>>,
        /// RES-088: span of the `assert` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-087: converted from tuple form so it can carry the span
    /// of the opening `{` (consumed in follow-ups).
    Block {
        stmts: Vec<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    LetStatement {
        name: String,
        value: Box<Node>,
        /// RES-052: optional type annotation, e.g. `let x: int = 0;`.
        /// Advisory today; enforced in RES-053.
        #[allow(dead_code)]
        type_annot: Option<String>,
        /// RES-079: span of the statement's originating source range.
        /// Currently unused at call sites; follow-ups will surface it
        /// in richer diagnostics (e.g. pointing at just the `let`
        /// keyword vs the whole enclosing statement).
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-013: `static let NAME = EXPR;` — like let, but stored in a
    /// per-interpreter statics map so the binding survives across
    /// function calls. First evaluation sets the value; subsequent
    /// evaluations are no-ops.
    StaticLet {
        name: String,
        value: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-017: re-bind an existing variable. Fails at runtime if the
    /// name has not been declared with `let` or `static let`.
    Assignment {
        name: String,
        value: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    ReturnStatement {
        /// `None` for a bare `return;`
        value: Option<Box<Node>>,
        #[allow(dead_code)]
        span: span::Span,
    },
    IfStatement {
        condition: Box<Node>,
        consequence: Box<Node>,
        alternative: Option<Box<Node>>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-023: `while COND { BODY }`. Body re-evaluated until COND is falsy.
    WhileStatement {
        condition: Box<Node>,
        body: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-037: `for NAME in EXPR { BODY }`. `EXPR` must evaluate to an
    /// array; `NAME` is bound to each element in order.
    ForInStatement {
        name: String,
        iterable: Box<Node>,
        body: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-087: converted from tuple form so it can carry a span
    /// matching the wrapped expression's starting token.
    ExpressionStatement {
        expr: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-078: identifiers carry source span so diagnostics can
    /// point at the referenced name.
    Identifier {
        name: String,
        span: span::Span,
    },
    /// RES-078: literal nodes carry source span so diagnostics
    /// (typechecker, verifier, VM runtime errors) can point at the
    /// offending value. The fields are unused today — RES-079 and
    /// RES-080 follow-ups will surface them in richer diagnostics —
    /// so the dead_code allow is deliberate and scoped.
    IntegerLiteral {
        value: i64,
        #[allow(dead_code)]
        span: span::Span,
    },
    FloatLiteral {
        value: f64,
        #[allow(dead_code)]
        span: span::Span,
    },
    StringLiteral {
        value: String,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-152: byte-string literal — `b"\x00\x01"`. Payload is the
    /// decoded `Vec<u8>` that the lexer produced from the source
    /// escape sequences. Value is cloned into `Value::Bytes` at
    /// eval time.
    BytesLiteral {
        value: Vec<u8>,
        #[allow(dead_code)]
        span: span::Span,
    },
    BooleanLiteral {
        value: bool,
        #[allow(dead_code)]
        span: span::Span,
    },
    PrefixExpression {
        operator: String,
        right: Box<Node>,
        /// RES-084: source span of the operator token. Consumed in
        /// follow-ups (e.g. typechecker arithmetic-mismatch errors).
        #[allow(dead_code)]
        span: span::Span,
    },
    InfixExpression {
        left: Box<Node>,
        operator: String,
        right: Box<Node>,
        /// RES-084: span of the operator token (NOT the full
        /// `lhs op rhs` range — that's a future refinement).
        #[allow(dead_code)]
        span: span::Span,
    },
    CallExpression {
        function: Box<Node>,
        arguments: Vec<Node>,
        /// RES-084: span of the call's `(` token. Used by future
        /// arity-mismatch and unknown-function diagnostics.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-041: `expr?` — if the operand is `Ok(v)`, evaluate to `v`;
    /// if `Err(e)`, return `Err(e)` from the enclosing function.
    ///
    /// RES-086: converted from tuple form so it can carry the span
    /// of the `?` operator (consumed in follow-ups).
    TryExpression {
        expr: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-042: anonymous fn expression. Unlike `Node::Function`, this
    /// node is not bound to a name — it evaluates to a `Value::Function`
    /// directly. Captures its defining env by value, matching existing
    /// named-fn semantics.
    FunctionLiteral {
        parameters: Vec<(String, String)>,
        body: Box<Node>,
        requires: Vec<Node>,
        ensures: Vec<Node>,
        #[allow(dead_code)]
        return_type: Option<String>,
        /// RES-088: span of the `fn` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-039: `match SCRUTINEE { PATTERN => EXPR, ... }` expression.
    ///
    /// RES-159: each arm now also carries an optional **guard**
    /// expression — `case <pattern> if <guard> => <body>` — evaluated
    /// in the pattern's binding scope. `None` is an unguarded arm;
    /// `Some(expr)` is re-evaluated per arm visit and falls through to
    /// the next arm on `false`.
    Match {
        scrutinee: Box<Node>,
        arms: Vec<(Pattern, Option<Node>, Node)>,
        /// RES-088: span of the `match` keyword. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-038: `struct NAME { TYPE FIELD, ... }` declaration. Fields
    /// are carried but currently unused at runtime — the typechecker
    /// (G7) will register them in a struct table to verify literal
    /// construction.
    #[allow(dead_code)]
    StructDecl {
        name: String,
        fields: Vec<(String, String)>, // (type, field_name)
        /// RES-088: span of the `struct` keyword. Consumed in follow-ups.
        span: span::Span,
    },
    /// RES-155: `let <StructName> { field1, field2: local, .. } = expr;`
    /// struct destructuring. `fields` holds `(field_name, local_name)`
    /// pairs; `local_name == field_name` when the shorthand form
    /// `{ x }` is used. `has_rest` marks the `..` trailing token:
    /// when true, fields not in the pattern are silently ignored;
    /// when false, the typechecker enforces exhaustiveness and
    /// errors listing any missing field names.
    LetDestructureStruct {
        struct_name: String,
        fields: Vec<(String, String)>, // (field_name, local_name)
        has_rest: bool,
        value: Box<Node>,
        /// RES-088: span of the `let` keyword.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-038: `NAME { field: expr, ... }` struct literal.
    StructLiteral {
        name: String,
        fields: Vec<(String, Node)>,
        /// RES-088: span of the type-name token. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-038: `target.field` read.
    FieldAccess {
        target: Box<Node>,
        field: String,
        /// RES-085: span of the `.field` operator. Consumed in
        /// follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-038: `target.field = expr` write.
    FieldAssignment {
        target: Box<Node>,
        field: String,
        value: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-032: `[e1, e2, e3]` array literal.
    ///
    /// RES-086: converted from tuple form so it can carry a span
    /// covering the opening `[` (span consumed in follow-ups).
    ArrayLiteral {
        items: Vec<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-032: `a[i]` read.
    IndexExpression {
        target: Box<Node>,
        index: Box<Node>,
        /// RES-085: span of the opening `[`. Consumed in follow-ups.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-032: `a[i] = expr` write.
    IndexAssignment {
        target: Box<Node>,
        index: Box<Node>,
        value: Box<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-148: map literal — `{"k" -> 1, "m" -> 2}`. Entries are
    /// (key_expr, value_expr) pairs; keys are evaluated at runtime
    /// and must produce one of the three hashable primitives
    /// (`Int`, `String`, `Bool`) or the interpreter errors.
    MapLiteral {
        entries: Vec<(Node, Node)>,
        /// Span of the opening `{`.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-149: set literal — `#{1, 2, 3}`. Items evaluate at runtime
    /// and must produce one of the three hashable primitives
    /// (`Int`, `String`, `Bool`) — same restriction as `MapLiteral`,
    /// enforced by `MapKey::from_value` (reused so the policy stays
    /// in one place).
    SetLiteral {
        items: Vec<Node>,
        /// Span of the opening `#{`.
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-158: `impl <StructName> { fn method(self, ...) { ... } ... }`.
    /// Methods are parsed as `Node::Function` values with pre-mangled
    /// names (`<StructName>$<method>`) and `self` injected as the first
    /// parameter typed as the enclosing struct. The interpreter and
    /// typechecker handle this variant by iterating `methods` and
    /// dispatching to each method as a regular top-level fn.
    ImplBlock {
        struct_name: String,
        methods: Vec<Node>,
        #[allow(dead_code)]
        span: span::Span,
    },
    /// RES-128: top-level `type <Name> = <Target>;` type alias.
    /// Aliases are structural, NOT nominal — `Meters` unifies with
    /// `Int` at every use site. For a fresh nominal type, declare a
    /// one-field struct instead (the ticket notes call this out).
    /// The typechecker maintains a `type_aliases` map populated
    /// from every `TypeAlias` statement and expands aliases
    /// transitively (with cycle detection) in `parse_type_name`.
    TypeAlias {
        name: String,
        target: String,
        #[allow(dead_code)]
        span: span::Span,
    },
}

// Parser for creating AST from tokens
struct Parser {
    lexer: Lexer,
    current_token: Token,
    peek_token: Token,
    /// Source position (line, column) of `current_token`. 1-indexed.
    current_line: usize,
    current_column: usize,
    /// Source position of `peek_token`.
    peek_line: usize,
    peek_column: usize,
    errors: Vec<String>,
    /// RES-156: fresh-name counter for array-comprehension
    /// desugaring. Each comprehension bumps this to mint a unique
    /// `_r$N` accumulator so nested / multiple comprehensions
    /// don't shadow each other's internal name (the `$` is not a
    /// legal identifier char in user code, so user bindings
    /// can't collide).
    comprehension_counter: u32,
}

impl Parser {
    fn new(lexer: Lexer) -> Self {
        let mut parser = Parser {
            lexer,
            current_token: Token::Eof,
            peek_token: Token::Eof,
            current_line: 1,
            current_column: 1,
            peek_line: 1,
            peek_column: 1,
            errors: Vec::new(),
            comprehension_counter: 0,
        };

        parser.next_token();
        parser.next_token();
        parser
    }

    /// Record an error, prefixing with the start of `current_token`
    /// so users see `line:col: Parser error: ...`.
    fn record_error(&mut self, msg: String) {
        let full = format!(
            "{}:{}: {}",
            self.current_line, self.current_column, msg
        );
        eprintln!("\x1B[31mParser error: {}\x1B[0m", full);
        self.errors.push(full);
    }

    fn next_token(&mut self) {
        self.current_token = self.peek_token.clone();
        self.current_line = self.peek_line;
        self.current_column = self.peek_column;
        self.peek_token = self.lexer.next_token();
        self.peek_line = self.lexer.last_token_line;
        self.peek_column = self.lexer.last_token_column;
    }
    
    fn parse_program(&mut self) -> Node {
        let mut program: Vec<span::Spanned<Node>> = Vec::new();

        while self.current_token != Token::Eof {
            // RES-077 (G6 partial): capture each statement's source
            // span by snapshotting the lexer's last_token_line/column
            // BEFORE parse_statement and AFTER. End-position reflects
            // the lexer's cursor at the moment the statement-recognizer
            // returned, which is close enough to the true end-of-stmt
            // for diagnostics (off by at most one whitespace token).
            let start = span::Pos::new(
                self.lexer.last_token_line,
                self.lexer.last_token_column,
                0,
            );
            if let Some(statement) = self.parse_statement() {
                let end = span::Pos::new(
                    self.lexer.last_token_line,
                    self.lexer.last_token_column,
                    0,
                );
                program.push(span::Spanned::new(
                    statement,
                    span::Span::new(start, end),
                ));
            }
            self.next_token();
        }

        Node::Program(program)
    }
    
    fn parse_statement(&mut self) -> Option<Node> {
        match self.current_token {
            // RES-191: `@pure` (and future attributes) prefix a
            // function declaration. Dispatched here so the
            // annotation + fn parse as a unit.
            Token::At => Some(self.parse_attributed_item()),
            Token::Function => Some(self.parse_function()),
            Token::Struct => Some(self.parse_struct_decl()),
            Token::Impl => Some(self.parse_impl_block()),
            Token::Type => Some(self.parse_type_alias()),
            Token::Use => self.parse_use_statement(),
            Token::Let => Some(self.parse_let_statement()),
            Token::Static => Some(self.parse_static_let_statement()),
            Token::Return => Some(self.parse_return_statement()),
            Token::Live => Some(self.parse_live_block()),
            Token::Assert => Some(self.parse_assert()),
            Token::If => Some(self.parse_if_statement()),
            Token::While => Some(self.parse_while_statement()),
            Token::For => Some(self.parse_for_in_statement()),
            Token::Unknown(ch) => {
                // RES-114: if the offending char is alphabetic (in
                // the Unicode sense), the lexer's ASCII-only
                // policy rejected it specifically as a non-ASCII
                // identifier candidate. Surface a dedicated
                // message so users grep for "non-ASCII" rather
                // than chasing a generic "Unexpected character".
                let msg = if ch.is_alphabetic() && !ch.is_ascii() {
                    format!(
                        "identifier contains non-ASCII character '{}' \
                         — Resilient identifiers are ASCII-only (see SYNTAX.md)",
                        ch
                    )
                } else {
                    format!("Unexpected character '{}'", ch)
                };
                self.record_error(msg);
                None
            }
            // Assignment: `IDENT = EXPR;` — disambiguated from an
            // expression statement by looking ahead for `=`.
            Token::Identifier(_) if self.peek_token == Token::Assign => {
                Some(self.parse_assignment())
            }
            // Index / field assignment: `IDENT[...] = EXPR;` or
            // `IDENT.field.more = EXPR;`. We let the expression parser
            // build the full LHS, then disambiguate at the `=`.
            Token::Identifier(_)
                if self.peek_token == Token::LeftBracket
                    || self.peek_token == Token::Dot =>
            {
                Some(self.parse_maybe_index_assignment())
            }
            _ => self.parse_expression_statement(),
        }
    }

    /// Parse either `IDENT[...] = EXPR;` (index assignment) or fall
    /// through to a plain expression statement if no `=` follows the
    /// index. Entered with current_token = the leading Identifier.
    fn parse_maybe_index_assignment(&mut self) -> Node {
        // Parse the index expression (which consumes IDENT, [, index, ]).
        let lhs = self
            .parse_expression(0)
            .unwrap_or(Node::IntegerLiteral { value: 0, span: span::Span::default() });
        // If this is an assignment, peek should be `=`.
        if self.peek_token == Token::Assign {
            self.next_token(); // move onto '='
            self.next_token(); // skip '=' to first token of RHS
            let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral { value: 0, span: span::Span::default() });
            if self.peek_token == Token::Semicolon {
                self.next_token();
            }
            // Destructure the LHS to pick the right assignment shape.
            // RES-085: pull span through so the Assignment node
            // inherits the LHS expression's span.
            match lhs {
                Node::IndexExpression { target, index, span } => Node::IndexAssignment {
                    target,
                    index,
                    value: Box::new(value),
                    span,
                },
                Node::FieldAccess { target, field, span } => Node::FieldAssignment {
                    target,
                    field,
                    value: Box::new(value),
                    span,
                },
                _ => Node::ExpressionStatement {
                    expr: Box::new(lhs),
                    span: span::Span::default(),
                },
            }
        } else {
            if self.peek_token == Token::Semicolon {
                self.next_token();
            }
            Node::ExpressionStatement {
                expr: Box::new(lhs),
                span: span::Span::default(),
            }
        }
    }

    fn parse_assignment(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => unreachable!("parse_assignment only dispatched for Identifier"),
        };
        self.next_token(); // move onto '='
        self.next_token(); // skip '=' to first token of RHS
        let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral { value: 0, span: span::Span::default() });
        if self.peek_token == Token::Semicolon {
            self.next_token();
        }
        Node::Assignment {
            name,
            value: Box::new(value),
            span: stmt_span,
        }
    }

    fn parse_function(&mut self) -> Node {
        // Default: no `@pure` annotation. The attribute-dispatch
        // path (see parse_attributed_item) calls
        // `parse_function_with_pure(true)` instead.
        self.parse_function_with_pure(false)
    }

    /// RES-191: parse an attribute prefix (`@pure`) followed by the
    /// attributed declaration. Only `@pure` is recognized today;
    /// future attributes (`@inline`, `@deprecated`, …) dispatch
    /// here. On entry `current_token` is `@`.
    ///
    /// Error-recovery strategy: if the attribute name is unknown,
    /// emit a diagnostic but continue — parse the following item
    /// without annotation. If the attributed item isn't a `fn`,
    /// same thing: diagnose and fall through to the generic
    /// statement parser. That way a user's in-flight file doesn't
    /// cascade into unrelated parse errors just because of a typo.
    fn parse_attributed_item(&mut self) -> Node {
        debug_assert_eq!(self.current_token, Token::At);
        self.next_token(); // skip '@'

        let attr_name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            other => {
                let tok = other.clone();
                self.record_error(format!(
                    "Expected attribute name after '@', found {}",
                    tok
                ));
                // Best-effort: ignore the broken attribute, try to
                // parse whatever follows as a normal statement.
                return self
                    .parse_statement()
                    .unwrap_or(Node::IntegerLiteral {
                        value: 0,
                        span: span::Span::default(),
                    });
            }
        };
        self.next_token(); // skip attribute name

        let pure_flag = match attr_name.as_str() {
            "pure" => true,
            other => {
                self.record_error(format!(
                    "Unknown attribute `@{}`. Known: @pure",
                    other
                ));
                // Fall through — treat as if no attribute was
                // present; the fn still parses.
                false
            }
        };

        // Only `fn` may be annotated today. Reject other targets
        // with a clear error.
        if self.current_token != Token::Function {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "@{} may only annotate a `fn` declaration, found {}",
                attr_name, tok
            ));
            // Best-effort recovery: parse whatever's next so the
            // rest of the file still parses.
            return self
                .parse_statement()
                .unwrap_or(Node::IntegerLiteral {
                    value: 0,
                    span: span::Span::default(),
                });
        }

        self.parse_function_with_pure(pure_flag)
    }

    /// RES-191: shared parser for `fn ...` with an explicit `pure`
    /// flag. Called from `parse_function` (no annotation → pure=false)
    /// and from `parse_attributed_item` when a `@pure` prefix
    /// precedes the `fn`.
    fn parse_function_with_pure(&mut self, pure: bool) -> Node {
        // RES-088: capture the `fn` keyword's span before advancing.
        let fn_span = self.span_at_current();
        self.next_token(); // Skip 'fn'

        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'fn', found {}", tok));
                // Return a placeholder to allow parsing to continue
                String::from("error_function")
            },
        };

        self.next_token(); // Skip name

        // Check if we have a left parenthesis as expected
        if self.current_token != Token::LeftParen {
            // For better error messages, provide more context
            if name == "main" {
                self.record_error(format!("Expected '(' after function name '{}'. Functions in Resilient must have parameters, even if unused. Try: fn main(int dummy) {{ ... }}", name));
            } else {
                self.record_error(format!("Expected '(' after function name '{}'", name));
            }

            // Try to recover by skipping to the opening brace
            while self.current_token != Token::LeftBrace && self.current_token != Token::Eof {
                self.next_token();
            }

            if self.current_token == Token::Eof {
                return Node::Function {
                    name,
                    parameters: Vec::new(),
                    body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                    requires: Vec::new(),
                    ensures: Vec::new(),
                    return_type: None,
                    span: fn_span,
                    pure,
                };
            }

            let body = self.parse_block_statement();
            return Node::Function {
                name,
                parameters: Vec::new(),
                body: Box::new(body),
                requires: Vec::new(),
                ensures: Vec::new(),
                return_type: None,
                span: fn_span,
                pure,
            };
        }

        self.next_token(); // Skip '('

        let parameters = self.parse_function_parameters();

        // RES-052: optional `-> TYPE` return type, BEFORE contracts.
        let return_type = self.parse_optional_return_type();

        // RES-035: between the parameter list and the body, accept any
        // number of `requires EXPR` and `ensures EXPR` clauses, in any
        // order. Each clause parses as a single expression.
        let (requires, ensures) = self.parse_function_contracts();

        if self.current_token != Token::LeftBrace {
            self.record_error(format!("Expected '{{' after function parameters for '{}'", name));
            // Try to recover by skipping to the opening brace
            while self.current_token != Token::LeftBrace && self.current_token != Token::Eof {
                self.next_token();
            }

            if self.current_token == Token::Eof {
                return Node::Function {
                    name,
                    parameters,
                    body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                    requires,
                    ensures,
                    return_type,
                    span: fn_span,
                    pure,
                };
            }
        }

        let body = self.parse_block_statement();

        Node::Function {
            name,
            parameters,
            body: Box::new(body),
            requires,
            ensures,
            return_type,
            span: fn_span,
            pure,
        }
    }

    /// RES-158: parse `impl <StructName> { <method_fn>* }`. Each
    /// inner method becomes a top-level `Node::Function` with the
    /// name mangled as `<StructName>$<method>` and `self` (if
    /// present as the first parameter) typed as the enclosing
    /// struct. The `ImplBlock` node the parser emits is deconstructed
    /// by the interpreter and typechecker back into its individual
    /// methods, so downstream stages see them as plain functions.
    fn parse_impl_block(&mut self) -> Node {
        let impl_span = self.span_at_current();
        self.next_token(); // skip 'impl'

        let struct_name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            other => {
                self.record_error(format!(
                    "Expected struct name after 'impl', found {}",
                    other
                ));
                String::new()
            }
        };
        self.next_token(); // skip name

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' after 'impl {}', found {}",
                struct_name, tok
            ));
        } else {
            self.next_token(); // skip '{'
        }

        let mut methods: Vec<Node> = Vec::new();
        while self.current_token != Token::RightBrace
            && self.current_token != Token::Eof
        {
            if self.current_token != Token::Function {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected 'fn' inside impl block, found {}",
                    tok
                ));
                // Best-effort recovery: skip ahead to the closing brace
                // so the whole parse doesn't cascade.
                while self.current_token != Token::RightBrace
                    && self.current_token != Token::Eof
                {
                    self.next_token();
                }
                break;
            }
            methods.push(self.parse_method(&struct_name));
            // `parse_block_statement` (called inside `parse_method` for
            // the body) leaves the cursor ON the method body's closing
            // `}`. Advance past it so the next iteration sees either
            // another `fn` or the impl block's own `}` — matching the
            // convention `parse_program` expects for its callers.
            if self.current_token == Token::RightBrace {
                self.next_token();
            }
        }
        // Leave the cursor ON the impl block's closing `}` — the outer
        // `parse_program` loop advances past each statement's final
        // token, same as with `fn` / `struct` decls.

        Node::ImplBlock { struct_name, methods, span: impl_span }
    }

    /// RES-158: parse a single method inside an `impl` block. Returns
    /// a `Node::Function` with the method name mangled
    /// (`<StructName>$<method>`) and `self` — if present as the
    /// first param — injected as `(<StructName>, "self")`.
    fn parse_method(&mut self, struct_name: &str) -> Node {
        let fn_span = self.span_at_current();
        self.next_token(); // skip 'fn'

        let method_name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            other => {
                self.record_error(format!(
                    "Expected method name after 'fn' in impl block, found {}",
                    other
                ));
                String::new()
            }
        };
        let mangled = format!("{}${}", struct_name, method_name);
        self.next_token(); // skip name

        if self.current_token != Token::LeftParen {
            self.record_error(format!(
                "Expected '(' after method name '{}'",
                method_name
            ));
        } else {
            self.next_token(); // skip '('
        }

        let mut parameters: Vec<(String, String)> = Vec::new();
        // Special-case the `self` first parameter: accept it bare
        // (no explicit type) and synthesize `(StructName, "self")`.
        if let Token::Identifier(name) = &self.current_token
            && name == "self"
        {
            parameters.push((struct_name.to_string(), "self".to_string()));
            self.next_token(); // skip 'self'
            if self.current_token == Token::Comma {
                self.next_token(); // skip ','
            }
        }

        // Parse any remaining TYPE NAME params until the `)`.
        if self.current_token != Token::RightParen {
            let rest = self.parse_function_parameters();
            parameters.extend(rest);
        } else {
            self.next_token(); // skip ')'
        }

        let return_type = self.parse_optional_return_type();
        let (requires, ensures) = self.parse_function_contracts();

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' after method signature for '{}', found {}",
                method_name, tok
            ));
        }
        let body = self.parse_block_statement();

        Node::Function {
            name: mangled,
            parameters,
            body: Box::new(body),
            requires,
            ensures,
            return_type,
            span: fn_span,
            // Impl methods inherit no annotation today. When
            // `@pure fn method(...)` is supported inside `impl`
            // blocks, this will take the method-level flag.
            pure: false,
        }
    }

    /// RES-128: parse `type <Name> = <Target>;` at top level. Emits
    /// a `Node::TypeAlias`. The target is parsed as a single
    /// identifier — tuple / generic alias targets are an RES-129
    /// follow-up. A missing `=`, a non-identifier on either side, or
    /// a missing `;` gets a clean diagnostic but doesn't stop the
    /// parser — we still emit the node so later passes don't null-
    /// pointer on missing metadata.
    fn parse_type_alias(&mut self) -> Node {
        let kw_span = self.span_at_current();
        self.next_token(); // skip `type`

        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            other => {
                self.record_error(format!(
                    "Expected alias name after 'type', found {}",
                    other
                ));
                String::new()
            }
        };
        self.next_token(); // skip name

        if self.current_token != Token::Assign {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '=' after 'type {}', found {}",
                name, tok
            ));
        } else {
            self.next_token(); // skip '='
        }

        let target = match &self.current_token {
            Token::Identifier(t) => t.clone(),
            other => {
                self.record_error(format!(
                    "Expected target type name after 'type {} =', found {}",
                    name, other
                ));
                String::new()
            }
        };
        self.next_token(); // skip target

        // Trailing `;` — optional (mirrors LetStatement's semicolon
        // handling so copy-paste doesn't trip users up).
        if self.current_token == Token::Semicolon {
            // leave cursor on `;`; parse_program advances past it
        } else if self.peek_token == Token::Semicolon {
            self.next_token();
        }

        Node::TypeAlias { name, target, span: kw_span }
    }

    /// Parse an optional `-> TYPE`. If present, current_token advances
    /// past the type identifier. If absent, no tokens are consumed.
    fn parse_optional_return_type(&mut self) -> Option<String> {
        if self.current_token != Token::Arrow {
            return None;
        }
        self.next_token(); // skip '->'
        let ty = match &self.current_token {
            Token::Identifier(t) => Some(t.clone()),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected type name after '->', found {}", tok));
                None
            }
        };
        self.next_token(); // skip type identifier
        ty
    }

    /// Parse zero or more `requires EXPR` / `ensures EXPR` clauses. On
    /// entry current_token is whatever followed the parameter list's
    /// `)`; on exit it's the `{` that starts the body (or whatever
    /// caused parsing to give up).
    fn parse_function_contracts(&mut self) -> (Vec<Node>, Vec<Node>) {
        let mut requires = Vec::new();
        let mut ensures = Vec::new();
        loop {
            match self.current_token {
                Token::Requires => {
                    self.next_token(); // skip `requires`
                    let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: true, span: span::Span::default() });
                    self.next_token(); // move past last token of expression
                    requires.push(expr);
                }
                Token::Ensures => {
                    self.next_token();
                    let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: true, span: span::Span::default() });
                    self.next_token();
                    ensures.push(expr);
                }
                _ => break,
            }
        }
        (requires, ensures)
    }
    
    fn parse_function_parameters(&mut self) -> Vec<(String, String)> {
        let mut parameters = Vec::new();
        
        if self.current_token == Token::RightParen {
            self.next_token(); // Skip ')'
            return parameters;
        }
        
        while self.current_token != Token::RightParen {
            let param_type = match &self.current_token {
                Token::Identifier(typ) => typ.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected parameter type, found {}", tok));
                    // Recover: bail out of the loop; caller will see RightParen
                    // or Eof and stop.
                    break;
                }
            };

            self.next_token(); // Skip type

            let param_name = match &self.current_token {
                Token::Identifier(name) => name.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected parameter name, found {}", tok));
                    break;
                }
            };

            parameters.push((param_type, param_name));

            self.next_token(); // Skip name

            if self.current_token == Token::Comma {
                self.next_token(); // Skip ','
            } else if self.current_token != Token::RightParen {
                // RES-118: multi-alternative form via `format_expected`.
                let tok_syntax = self.current_token.display_syntax();
                self.record_error(format!(
                    "after parameter: {}",
                    format_expected(&["`,`", "`)`"], &tok_syntax)
                ));
                break;
            }
        }
        
        self.next_token(); // Skip ')'
        parameters
    }
    
    fn parse_block_statement(&mut self) -> Node {
        // RES-087: capture the `{` token's span before advancing.
        let brace_span = self.span_at_current();
        let mut statements = Vec::new();

        self.next_token(); // Skip '{'

        while self.current_token != Token::RightBrace && self.current_token != Token::Eof {
            if let Some(stmt) = self.parse_statement() {
                statements.push(stmt);
            }
            self.next_token();
        }

        Node::Block { stmts: statements, span: brace_span }
    }
    
    /// `static let NAME = EXPR;` — parsed into a StaticLet node. The
    /// implementation just reuses parse_let_statement after consuming
    /// the `static` keyword and enforcing that `let` follows.
    /// `for NAME in EXPR { BODY }`
    fn parse_for_in_statement(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'for'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'for', found {}", tok));
                String::new()
            }
        };
        self.next_token(); // skip name
        if self.current_token != Token::In {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected 'in' after 'for {}', found {}", name, tok));
            return Node::ForInStatement {
                name,
                iterable: Box::new(Node::ArrayLiteral { items: Vec::new(), span: span::Span::default() }),
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                span: stmt_span,
            };
        }
        self.next_token(); // skip 'in'
        let iterable = self.parse_expression(0).unwrap_or(Node::ArrayLiteral { items: Vec::new(), span: span::Span::default() });
        self.next_token(); // advance past the expression's tail (RES-014 invariant)

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after for-iterable, found {}", tok));
            return Node::ForInStatement {
                name,
                iterable: Box::new(iterable),
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                span: stmt_span,
            };
        }
        let body = self.parse_block_statement();
        Node::ForInStatement {
            name,
            iterable: Box::new(iterable),
            body: Box::new(body),
            span: stmt_span,
        }
    }

    /// `while COND { BODY }` — same parsing shape as `if` (both `while (c)`
    /// and `while c` forms), minus the `else` branch.
    fn parse_while_statement(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'while'

        let condition = if self.current_token == Token::LeftParen {
            self.next_token();
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: false, span: span::Span::default() });
            self.next_token();
            if self.current_token != Token::RightParen {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected ')' after while condition, found {}", tok));
            } else {
                self.next_token();
            }
            expr
        } else {
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: false, span: span::Span::default() });
            self.next_token();
            expr
        };

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after while condition, found {}", tok));
            return Node::WhileStatement {
                condition: Box::new(condition),
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                span: stmt_span,
            };
        }

        let body = self.parse_block_statement();
        Node::WhileStatement {
            condition: Box::new(condition),
            body: Box::new(body),
            span: stmt_span,
        }
    }

    fn parse_static_let_statement(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'static'
        if self.current_token != Token::Let {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected 'let' after 'static', found {}", tok));
            return Node::StaticLet {
                name: String::new(),
                value: Box::new(Node::IntegerLiteral { value: 0, span: span::Span::default() }),
                span: stmt_span,
            };
        }
        // Delegate to parse_let_statement and re-wrap. parse_let_statement
        // returns a Node::LetStatement.
        let inner = self.parse_let_statement();
        match inner {
            Node::LetStatement { name, value, span, .. } => Node::StaticLet { name, value, span },
            other => other, // error paths return a degenerate LetStatement
        }
    }

    fn parse_let_statement(&mut self) -> Node {
        // RES-079: capture span of the `let` keyword BEFORE we
        // advance. The parse method's end position would otherwise
        // be wherever the lexer landed after the semicolon — the
        // wrong thing to attribute the statement to.
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'let'

        let name = match &self.current_token {
            Token::Identifier(name) => name.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'let', found {}", tok));
                return Node::LetStatement {
                    name: String::new(),
                    value: Box::new(Node::IntegerLiteral { value: 0, span: span::Span::default() }),
                    type_annot: None,
                    span: stmt_span,
                };
            }
        };

        self.next_token(); // Skip name

        // RES-155: struct destructuring form — `let <StructName> { ... } = expr;`.
        // The `{` immediately after an identifier (no `:` or `=`) is
        // unambiguous: the simple-let and annotated-let forms both
        // require those tokens next. We reroute here and let the
        // dedicated parser take over.
        if self.current_token == Token::LeftBrace {
            return self.parse_let_destructure_struct(name, stmt_span);
        }

        // RES-052: optional `: TYPE` annotation.
        let type_annot = if self.current_token == Token::Colon {
            self.next_token(); // skip ':'
            let ty = match &self.current_token {
                Token::Identifier(t) => Some(t.clone()),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected type name after ':', found {}", tok));
                    None
                }
            };
            self.next_token(); // skip type
            ty
        } else {
            None
        };

        if self.current_token != Token::Assign {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '=' after identifier '{}' in let statement, found {}",
                name, tok
            ));
            return Node::LetStatement {
                name,
                value: Box::new(Node::IntegerLiteral { value: 0, span: span::Span::default() }),
                type_annot,
                span: stmt_span,
            };
        }

        self.next_token(); // Skip '='

        let value = self.parse_expression(0).unwrap();

        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }

        Node::LetStatement {
            name,
            value: Box::new(value),
            type_annot,
            span: stmt_span,
        }
    }

    /// RES-155: parse a struct-destructure `let` —
    /// `let <StructName> { field1, field2: local, .. } = expr;`.
    /// On entry, `current_token` is `{` (the caller has already
    /// consumed `let` and the `StructName` identifier). On exit,
    /// `current_token` sits on the last token of the value
    /// expression; `parse_statement` handles the trailing `;`.
    fn parse_let_destructure_struct(
        &mut self,
        struct_name: String,
        stmt_span: span::Span,
    ) -> Node {
        self.next_token(); // skip `{`
        let mut fields: Vec<(String, String)> = Vec::new();
        let mut has_rest = false;

        loop {
            // Trailing `}` (empty or end of list).
            if self.current_token == Token::RightBrace {
                break;
            }

            // Rest pattern `..` — must be the last element before `}`.
            // We accept it mid-list leniently but don't reorder.
            if self.current_token == Token::Dot {
                if self.peek_token == Token::Dot {
                    self.next_token(); // second `.`
                    has_rest = true;
                    self.next_token(); // advance past `..` to `,` or `}`
                    if self.current_token == Token::Comma {
                        self.next_token(); // trailing `,` after `..`
                    }
                    if self.current_token != Token::RightBrace {
                        let tok = self.current_token.clone();
                        self.record_error(format!(
                            "After `..` rest pattern, expected `}}`, found {}",
                            tok
                        ));
                    }
                    break;
                } else {
                    self.record_error(
                        "Expected `..` (two dots) for rest pattern, found single `.`"
                            .to_string(),
                    );
                    break;
                }
            }

            let field_name = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected field name in struct destructure, found {}",
                        tok
                    ));
                    break;
                }
            };
            self.next_token(); // advance past field name

            // Optional `: local_name` rename.
            let local_name = if self.current_token == Token::Colon {
                self.next_token(); // skip `:`
                let local = match &self.current_token {
                    Token::Identifier(n) => n.clone(),
                    _ => {
                        let tok = self.current_token.clone();
                        self.record_error(format!(
                            "Expected local binding name after `:`, found {}",
                            tok
                        ));
                        break;
                    }
                };
                self.next_token(); // advance past local name
                local
            } else {
                // Shorthand: field binds to a local of the same name,
                // matching RES-154's struct-literal shorthand on the
                // construction side.
                field_name.clone()
            };

            fields.push((field_name, local_name));

            if self.current_token == Token::Comma {
                self.next_token();
                continue;
            }
            if self.current_token == Token::RightBrace {
                break;
            }
            let tok_syntax = self.current_token.display_syntax();
            self.record_error(format!(
                "in struct destructure: {}",
                format_expected(&["`,`", "`}`", "`..`"], &tok_syntax)
            ));
            break;
        }

        // Consume the closing `}`.
        if self.current_token != Token::RightBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected `}}` to close struct destructure, found {}",
                tok
            ));
        } else {
            self.next_token(); // past `}`
        }

        // Now the `=` and value expression.
        if self.current_token != Token::Assign {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected `=` after struct destructure pattern, found {}",
                tok
            ));
            return Node::LetDestructureStruct {
                struct_name,
                fields,
                has_rest,
                value: Box::new(Node::IntegerLiteral {
                    value: 0,
                    span: span::Span::default(),
                }),
                span: stmt_span,
            };
        }
        self.next_token(); // skip `=`
        let value = self
            .parse_expression(0)
            .unwrap_or(Node::IntegerLiteral {
                value: 0,
                span: span::Span::default(),
            });

        if self.peek_token == Token::Semicolon {
            self.next_token();
        }

        Node::LetDestructureStruct {
            struct_name,
            fields,
            has_rest,
            value: Box::new(value),
            span: stmt_span,
        }
    }

    /// RES-073: `use "path/to/file.res";` — emits `Node::Use { path, span }`.
    /// Resolved by `imports::expand_uses` before typechecker / interpreter.
    fn parse_use_statement(&mut self) -> Option<Node> {
        // Caller checked self.current_token == Token::Use.
        self.next_token(); // consume 'use'
        let path = match &self.current_token {
            Token::StringLiteral(s) => s.clone(),
            _ => {
                self.record_error(
                    "Expected string literal after 'use' (e.g. `use \"helpers.res\";`)"
                        .to_string(),
                );
                return None;
            }
        };
        if self.peek_token == Token::Semicolon {
            self.next_token();
        }
        Some(Node::Use { path,
            span: self.span_at_current() })
    }

    fn parse_return_statement(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'return'

        // Bare `return;` or `return}` → no expression.
        if matches!(
            self.current_token,
            Token::Semicolon | Token::RightBrace | Token::Eof
        ) {
            return Node::ReturnStatement { value: None, span: stmt_span };
        }

        let value = match self.parse_expression(0) {
            Some(expr) => Some(Box::new(expr)),
            None => {
                self.record_error(
                    "Expected expression after 'return' (or write 'return;' for no value)".to_string()
                );
                None
            }
        };

        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }

        Node::ReturnStatement { value, span: stmt_span }
    }
    
    fn parse_live_block(&mut self) -> Node {
        self.next_token(); // Skip 'live'

        // RES-139 + RES-142: optional `backoff(...)` and `within
        // <duration>` clauses. Both are context-sensitive identifiers
        // (no reserved words burned); either order is accepted, but
        // neither may appear twice. Loop until we hit `invariant` or
        // `{`.
        let mut backoff: Option<BackoffConfig> = None;
        let mut timeout: Option<Box<Node>> = None;
        loop {
            match &self.current_token {
                Token::Identifier(n) if n == "backoff" => {
                    if backoff.is_some() {
                        self.record_error(
                            "duplicate `backoff(...)` clause in live block"
                                .to_string(),
                        );
                    }
                    let cfg = self.parse_backoff_kwargs();
                    if backoff.is_none() {
                        backoff = Some(cfg);
                    }
                }
                Token::Identifier(n) if n == "within" => {
                    if timeout.is_some() {
                        self.record_error(
                            "duplicate `within <duration>` clause in live block"
                                .to_string(),
                        );
                    }
                    let dl = self.parse_within_clause();
                    if timeout.is_none() {
                        timeout = dl.map(Box::new);
                    }
                }
                _ => break,
            }
        }

        // RES-036: zero or more `invariant EXPR` clauses between `live`
        // and `{`.
        let mut invariants = Vec::new();
        while self.current_token == Token::Invariant {
            self.next_token(); // skip `invariant`
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: true, span: span::Span::default() });
            self.next_token(); // move past last token of the expression
            invariants.push(expr);
        }

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after 'live', found {}", tok));
            return Node::LiveBlock {
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                invariants,
                backoff,
                timeout,
                span: self.span_at_current(),
            };
        }

        let body = self.parse_block_statement();

        Node::LiveBlock {
            body: Box::new(body),
            invariants,
            backoff,
            timeout,
            span: self.span_at_current()
        }
    }

    /// RES-142: parse `within <integer><unit>` into a
    /// `Node::DurationLiteral`. On entry, `current_token` is the
    /// `within` identifier. On exit, `current_token` sits on whatever
    /// follows the unit token. Unit ∈ {`ns`, `us`, `ms`, `s`}.
    ///
    /// Returns `None` on parse error (integer missing, unit missing or
    /// unknown, negative literal). Errors are recorded via
    /// `record_error` so downstream parsing stays productive.
    fn parse_within_clause(&mut self) -> Option<Node> {
        let start_span = self.span_at_current();
        self.next_token(); // skip `within`

        let raw = match &self.current_token {
            Token::IntLiteral(n) if *n >= 0 => *n as u64,
            other => {
                self.record_error(format!(
                    "Expected non-negative integer literal after `within`, found {}",
                    other
                ));
                return None;
            }
        };
        self.next_token(); // skip integer literal

        let unit = match &self.current_token {
            Token::Identifier(u) => u.clone(),
            other => {
                self.record_error(format!(
                    "Expected duration unit (`ns`, `us`, `ms`, `s`) after `within {}`, found {}",
                    raw, other
                ));
                return None;
            }
        };
        let per_unit_ns: u64 = match unit.as_str() {
            "ns" => 1,
            "us" => 1_000,
            "ms" => 1_000_000,
            "s"  => 1_000_000_000,
            other => {
                self.record_error(format!(
                    "Unknown duration unit `{}` — expected one of `ns`, `us`, `ms`, `s`",
                    other
                ));
                return None;
            }
        };
        self.next_token(); // skip unit

        // `saturating_mul` guards against overflow on absurd values
        // like `within 999999999999999999s` — we cap at u64::MAX,
        // effectively "no budget" (the runtime check will never
        // trip).
        let nanos = raw.saturating_mul(per_unit_ns);

        Some(Node::DurationLiteral { nanos, span: start_span })
    }

    /// RES-139: parse `backoff(base_ms=N, factor=K, max_ms=M)` —
    /// each kwarg optional with ticket defaults (1 / 2 / 100). On
    /// entry, `current_token` is the `backoff` identifier. On exit,
    /// `current_token` sits on whatever follows the closing `)`.
    ///
    /// Parse errors (missing `(`, non-int literal, `factor > 10`,
    /// unknown kwarg) `record_error` with a clean diagnostic; we
    /// then keep going with a best-effort default so the caller
    /// can still parse the `{ ... }` body without cascading.
    fn parse_backoff_kwargs(&mut self) -> BackoffConfig {
        let mut cfg = BackoffConfig::default_ticket();
        self.next_token(); // skip `backoff`
        if self.current_token != Token::LeftParen {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '(' after 'backoff', found {}",
                tok
            ));
            return cfg;
        }
        self.next_token(); // skip '('

        let mut first = true;
        while self.current_token != Token::RightParen && self.current_token != Token::Eof {
            if !first {
                if self.current_token != Token::Comma {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected ',' or ')' in backoff args, found {}",
                        tok
                    ));
                    break;
                }
                self.next_token(); // skip ','
            }
            first = false;

            // kwarg name
            let name = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                other => {
                    self.record_error(format!(
                        "Expected backoff kwarg name (`base_ms`, `factor`, `max_ms`), found {}",
                        other
                    ));
                    break;
                }
            };
            self.next_token(); // skip name

            if self.current_token != Token::Assign {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected '=' after backoff kwarg `{}`, found {}",
                    name, tok
                ));
                break;
            }
            self.next_token(); // skip '='

            // kwarg value — integer literal only.
            let value = match &self.current_token {
                Token::IntLiteral(n) if *n >= 0 => *n as u64,
                other => {
                    self.record_error(format!(
                        "Expected non-negative integer literal for backoff.`{}`, found {}",
                        name, other
                    ));
                    break;
                }
            };
            self.next_token(); // skip value

            match name.as_str() {
                "base_ms" => cfg.base_ms = value,
                "factor" => {
                    if value > 10 {
                        self.record_error(format!(
                            "backoff `factor` must be <= 10 (got {}) — larger values risk runaway sleeps on flaky hardware",
                            value
                        ));
                    } else {
                        cfg.factor = value;
                    }
                }
                "max_ms" => cfg.max_ms = value,
                other => {
                    self.record_error(format!(
                        "unknown backoff kwarg `{}` — expected one of `base_ms`, `factor`, `max_ms`",
                        other
                    ));
                }
            }
        }

        if self.current_token == Token::RightParen {
            self.next_token(); // skip ')'
        }
        cfg
    }
    
    fn parse_assert(&mut self) -> Node {
        self.next_token(); // Skip 'assert'
        
        if self.current_token != Token::LeftParen {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '(' after 'assert', found {}", tok));
            return Node::Assert {
                condition: Box::new(Node::BooleanLiteral { value: true, span: span::Span::default() }),
                message: None,
                span: self.span_at_current(),
            };
        }

        self.next_token(); // Skip '('

        let condition = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: true, span: span::Span::default() });
        self.next_token(); // RES-014: advance past last token of expression

        let message = if self.current_token == Token::Comma {
            self.next_token(); // Skip ','
            let msg = self.parse_expression(0).unwrap_or(Node::StringLiteral { value: String::new(), span: span::Span::default() });
            self.next_token(); // advance past last token of message expression
            Some(Box::new(msg))
        } else {
            None
        };

        if self.current_token != Token::RightParen {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected ')' after assert condition, found {}",
                tok
            ));
        }
        
        Node::Assert {
            condition: Box::new(condition),
            message,
            span: self.span_at_current()
        }
    }
    
    fn parse_if_statement(&mut self) -> Node {
        let stmt_span = self.span_at_current();
        self.next_token(); // Skip 'if'

        // Handle both `if (condition)` and `if condition` forms.
        //
        // RES-014 invariant note: `parse_expression` leaves `current_token`
        // pointing at the *last token it consumed*. So after the call we
        // must advance once to move past the expression's tail.
        let condition = if self.current_token == Token::LeftParen {
            self.next_token(); // Skip '('
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: false, span: span::Span::default() });
            self.next_token(); // Advance past last-token-of-expression

            if self.current_token != Token::RightParen {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ')' after if condition, found {}",
                    tok
                ));
            } else {
                self.next_token(); // Skip ')'
            }
            expr
        } else {
            let expr = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: false, span: span::Span::default() });
            self.next_token(); // Advance past last-token-of-expression
            expr
        };

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' after if condition, found {}",
                tok
            ));
            // Recover by returning a skeleton `if` with an empty body so
            // the rest of the file can still be parsed.
            return Node::IfStatement {
                condition: Box::new(condition),
                consequence: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                alternative: None,
                span: stmt_span,
            };
        }

        let consequence = self.parse_block_statement();

        let alternative = if self.peek_token == Token::Else {
            self.next_token(); // Move to 'else'
            self.next_token(); // Skip 'else'

            if self.current_token != Token::LeftBrace {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected '{{' after 'else', found {}", tok));
                None
            } else {
                Some(Box::new(self.parse_block_statement()))
            }
        } else {
            None
        };

        Node::IfStatement {
            condition: Box::new(condition),
            consequence: Box::new(consequence),
            alternative,
            span: stmt_span,
        }
    }

    fn parse_expression_statement(&mut self) -> Option<Node> {
        // RES-087: capture the span at the statement's first token
        // before parse_expression advances past it.
        let stmt_span = self.span_at_current();
        let expr = self.parse_expression(0)?;

        if self.peek_token == Token::Semicolon {
            self.next_token(); // Skip to semicolon
        }

        Some(Node::ExpressionStatement { expr: Box::new(expr), span: stmt_span })
    }
    
    /// RES-078: build a single-position `Span` from the lexer's
    /// current `last_token_*` state — good enough for leaf nodes
    /// where the "source range" is just wherever the token starts.
    fn span_at_current(&self) -> span::Span {
        let pos = span::Pos::new(
            self.lexer.last_token_line,
            self.lexer.last_token_column,
            0,
        );
        span::Span::new(pos, pos)
    }

    fn parse_expression(&mut self, precedence: u8) -> Option<Node> {
        // Parse prefix expressions
        let tok_span = self.span_at_current();
        let mut left_expr = match &self.current_token {
            Token::Identifier(name) => Some(Node::Identifier { name: name.clone(), span: tok_span }),
            Token::IntLiteral(value) => Some(Node::IntegerLiteral { value: *value, span: tok_span }),
            Token::FloatLiteral(value) => Some(Node::FloatLiteral { value: *value, span: tok_span }),
            Token::StringLiteral(value) => Some(Node::StringLiteral { value: value.clone(), span: tok_span }),
            // RES-152: byte-string literal, lexed to Vec<u8>.
            Token::BytesLiteral(value) => Some(Node::BytesLiteral { value: value.clone(), span: tok_span }),
            Token::BoolLiteral(value) => Some(Node::BooleanLiteral { value: *value, span: tok_span }),
            // RES-012: prefix operators `!` and `-`. Precedence is higher
            // than any infix operator, so the operand consumes only the
            // tightest-binding next expression.
            Token::Bang | Token::Minus => {
                let op = if self.current_token == Token::Bang { "!" } else { "-" };
                // RES-084: capture the operator's span before
                // advancing past it.
                let op_span = self.span_at_current();
                self.next_token();
                // Prefix precedence is higher than any infix operator
                // so `-1 + 2` parses as `(-1) + 2`, not `-(1 + 2)`.
                let right = self.parse_expression(11)?;
                Some(Node::PrefixExpression {
                    operator: op.to_string(),
                    right: Box::new(right),
                    span: op_span,
                })
            }
            Token::LeftParen => {
                self.next_token(); // Skip '('
                let expr = self.parse_expression(0);
                if self.current_token != Token::RightParen {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected ')' closing parenthesized expression, found {}",
                        tok
                    ));
                }
                expr
            },
            Token::LeftBracket => Some(self.parse_array_literal()),
            // RES-148: `{"k" -> v, ...}` in expression position parses
            // as a Map literal. Map literals are only valid in
            // expression context; statement-level `{` still starts a
            // block (e.g. fn body, live body, if/else blocks) and is
            // parsed by the statement-level machinery, which never
            // calls `parse_expression` at a `{` token.
            Token::LeftBrace => Some(self.parse_map_literal()),
            // RES-149: `#{1, 2, ...}` set literal. The dedicated
            // opener token sidesteps any ambiguity with `{` (map /
            // block) without needing the parser to peek ahead.
            Token::HashLeftBrace => Some(self.parse_set_literal()),
            Token::New => Some(self.parse_struct_literal()),
            Token::Match => Some(self.parse_match_expression()),
            Token::Function => Some(self.parse_function_literal()),
            _ => None,
        };
        
        // Parse infix expressions
        while self.peek_token != Token::Semicolon && precedence < self.peek_precedence() {
            let Some(current_left) = left_expr else {
                // No prefix expression to build on; stop trying to
                // fold infix operators into nothing.
                return None;
            };
            left_expr = match &self.peek_token {
                Token::Plus | Token::Minus | Token::Multiply | Token::Divide | Token::Modulo |
                Token::Equal | Token::NotEqual | Token::Less | Token::Greater |
                Token::LessEqual | Token::GreaterEqual | Token::And | Token::Or |
                Token::BitAnd | Token::BitOr | Token::BitXor |
                Token::ShiftLeft | Token::ShiftRight => {
                    self.next_token();
                    self.parse_infix_expression(current_left)
                },
                Token::LeftParen => {
                    self.next_token();
                    self.parse_call_expression(current_left)
                },
                Token::LeftBracket => {
                    self.next_token(); // move onto '['
                    self.parse_index_expression(current_left)
                },
                Token::Dot => {
                    self.next_token(); // move onto '.'
                    self.parse_field_access(current_left)
                },
                Token::Question => {
                    // Postfix `?` — consume it and wrap.
                    // RES-086: capture the `?` token's span before
                    // advancing past it.
                    let q_span = self.span_at_current();
                    self.next_token();
                    Some(Node::TryExpression {
                        expr: Box::new(current_left),
                        span: q_span,
                    })
                },
                _ => Some(current_left),
            };
        }
        
        left_expr
    }
    
    fn parse_infix_expression(&mut self, left: Node) -> Option<Node> {
        let operator = match &self.current_token {
            Token::Plus => "+".to_string(),
            Token::Minus => "-".to_string(),
            Token::Multiply => "*".to_string(),
            Token::Divide => "/".to_string(),
            Token::Modulo => "%".to_string(),
            Token::And => "&&".to_string(),
            Token::Or => "||".to_string(),
            Token::BitAnd => "&".to_string(),
            Token::BitOr => "|".to_string(),
            Token::BitXor => "^".to_string(),
            Token::ShiftLeft => "<<".to_string(),
            Token::ShiftRight => ">>".to_string(),
            Token::Equal => "==".to_string(),
            Token::NotEqual => "!=".to_string(),
            Token::Less => "<".to_string(),
            Token::Greater => ">".to_string(),
            Token::LessEqual => "<=".to_string(),
            Token::GreaterEqual => ">=".to_string(),
            _ => {
                // Unreachable in practice (the caller only dispatches
                // known operator tokens), but better to report than panic.
                let tok = self.current_token.clone();
                self.record_error(format!("Internal: unexpected operator token {:?}", tok));
                return None;
            }
        };
        
        let precedence = self.current_precedence();
        // RES-084: capture the operator's span before advancing.
        let op_span = self.span_at_current();
        self.next_token();

        let right = self.parse_expression(precedence).unwrap();

        Some(Node::InfixExpression {
            left: Box::new(left),
            operator,
            right: Box::new(right),
            span: op_span,
        })
    }

    fn parse_call_expression(&mut self, function: Node) -> Option<Node> {
        // RES-084: capture the call's span (lands on the `(` token)
        // before parsing arguments advances the lexer.
        let call_span = self.span_at_current();
        let arguments = self.parse_call_arguments();

        Some(Node::CallExpression {
            function: Box::new(function),
            arguments,
            span: call_span,
        })
    }
    
    fn parse_call_arguments(&mut self) -> Vec<Node> {
        let mut args = Vec::new();
        
        if self.peek_token == Token::RightParen {
            self.next_token();
            return args;
        }
        
        self.next_token();
        args.push(self.parse_expression(0).unwrap());
        
        while self.peek_token == Token::Comma {
            self.next_token(); // Skip current
            self.next_token(); // Skip comma
            args.push(self.parse_expression(0).unwrap());
        }
        
        if self.peek_token != Token::RightParen {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ')' after call arguments, found {}",
                tok
            ));
        } else {
            self.next_token(); // Skip to ')'
        }

        args
    }

    /// Parse `struct NAME { TYPE FIELD, ... }`. current_token is `struct`
    /// on entry; on exit current_token is `}`.
    fn parse_struct_decl(&mut self) -> Node {
        self.next_token(); // skip 'struct'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected identifier after 'struct', found {}", tok));
                String::new()
            }
        };
        self.next_token();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after struct name, found {}", tok));
            return Node::StructDecl { name, fields: Vec::new(),
            span: self.span_at_current() };
        }
        self.next_token(); // skip '{'

        let mut fields = Vec::new();
        while self.current_token != Token::RightBrace && self.current_token != Token::Eof {
            let ty = match &self.current_token {
                Token::Identifier(t) => t.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected field type, found {}", tok));
                    break;
                }
            };
            self.next_token();
            let fname = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!("Expected field name after type '{}', found {}", ty, tok));
                    break;
                }
            };
            fields.push((ty, fname));
            self.next_token();
            if self.current_token == Token::Comma {
                self.next_token();
            } else if self.current_token != Token::RightBrace {
                // RES-118: multi-alternative form via `format_expected`.
                let tok_syntax = self.current_token.display_syntax();
                self.record_error(format!(
                    "after struct field: {}",
                    format_expected(&["`,`", "`}`"], &tok_syntax)
                ));
                break;
            }
        }
        Node::StructDecl { name, fields,
            span: self.span_at_current() }
    }

    /// Parse `new NAME { field: expr, ... }`. current_token is `new`
    /// on entry; on exit current_token is `}`.
    fn parse_struct_literal(&mut self) -> Node {
        self.next_token(); // skip 'new'
        let name = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected struct name after 'new', found {}", tok));
                String::new()
            }
        };
        self.next_token();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after struct name, found {}", tok));
            return Node::StructLiteral { name, fields: Vec::new(),
            span: self.span_at_current() };
        }

        let mut fields: Vec<(String, Node)> = Vec::new();

        if self.peek_token == Token::RightBrace {
            self.next_token(); // to '}'
            return Node::StructLiteral { name, fields,
            span: self.span_at_current() };
        }

        self.next_token(); // skip '{'
        loop {
            // Capture the span of the field-name token so a
            // shorthand expansion's `Identifier` carries the
            // correct source position (the original field name's
            // location, not some synthetic blank span).
            let fname_span = self.span_at_current();
            let fname = match &self.current_token {
                Token::Identifier(n) => n.clone(),
                _ => {
                    let tok = self.current_token.clone();
                    self.record_error(format!(
                        "Expected field name in struct literal, found {}",
                        tok
                    ));
                    break;
                }
            };
            self.next_token();
            // RES-154: shorthand desugar. `Point { x, y }` expands
            // to `Point { x: x, y: y }` — if the field name is
            // followed directly by `,` or `}` instead of `:`, we
            // synthesize an `Identifier` value referring to the
            // same name. The typechecker / interpreter stay
            // ignorant of the sugar; unknown-identifier diagnostics
            // surface naturally if the name isn't bound in scope.
            if self.current_token == Token::Comma
                || self.current_token == Token::RightBrace
            {
                let value = Node::Identifier {
                    name: fname.clone(),
                    span: fname_span,
                };
                fields.push((fname, value));
                if self.current_token == Token::Comma {
                    self.next_token();
                    if self.current_token == Token::RightBrace {
                        break; // trailing-comma-before-`}`
                    }
                    continue; // another field follows
                } else {
                    break; // `}` closes the literal
                }
            }
            if self.current_token != Token::Colon {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected ':' after field name '{}' in struct literal, found {}",
                    fname, tok
                ));
                break;
            }
            self.next_token(); // skip ':'
            let value = self.parse_expression(0).unwrap_or(Node::IntegerLiteral { value: 0, span: span::Span::default() });
            fields.push((fname, value));
            // parse_expression leaves current on the last token of the
            // expression; advance to move past it.
            self.next_token();
            if self.current_token == Token::Comma {
                self.next_token();
                // Trailing comma before } is allowed.
                if self.current_token == Token::RightBrace {
                    break;
                }
            } else if self.current_token == Token::RightBrace {
                break;
            } else {
                // RES-118: multi-alternative form via `format_expected`.
                let tok_syntax = self.current_token.display_syntax();
                self.record_error(format!(
                    "in struct literal: {}",
                    format_expected(&["`,`", "`}`"], &tok_syntax)
                ));
                break;
            }
        }
        Node::StructLiteral { name, fields,
            span: self.span_at_current() }
    }

    /// Parse an anonymous `fn(params) -> TYPE? requires/ensures? { body }`.
    fn parse_function_literal(&mut self) -> Node {
        self.next_token(); // skip 'fn'
        if self.current_token != Token::LeftParen {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '(' after anonymous 'fn', found {}",
                tok
            ));
            return Node::FunctionLiteral {
                parameters: Vec::new(),
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                requires: Vec::new(),
                ensures: Vec::new(),
                return_type: None,
                span: self.span_at_current(),
            };
        }
        self.next_token(); // skip '('
        let parameters = self.parse_function_parameters();
        let return_type = self.parse_optional_return_type();
        let (requires, ensures) = self.parse_function_contracts();
        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected '{{' in anonymous fn, found {}",
                tok
            ));
            return Node::FunctionLiteral {
                parameters,
                body: Box::new(Node::Block { stmts: Vec::new(), span: span::Span::default() }),
                requires,
                ensures,
                return_type,
                span: self.span_at_current(),
            };
        }
        let body = self.parse_block_statement();
        Node::FunctionLiteral {
            parameters,
            body: Box::new(body),
            requires,
            ensures,
            return_type,
            span: self.span_at_current()
        }
    }

    /// Parse `match SCRUTINEE { PATTERN => EXPR, ... }`. Current token
    /// is `match` on entry; on exit it's `}`.
    fn parse_match_expression(&mut self) -> Node {
        self.next_token(); // skip 'match'
        let scrutinee = self.parse_expression(0).unwrap_or(Node::BooleanLiteral { value: false, span: span::Span::default() });
        self.next_token(); // past last token of scrutinee

        if self.current_token != Token::LeftBrace {
            let tok = self.current_token.clone();
            self.record_error(format!("Expected '{{' after match scrutinee, found {}", tok));
            return Node::Match {
                scrutinee: Box::new(scrutinee),
                arms: Vec::new(),
            span: self.span_at_current()
            };
        }

        let mut arms: Vec<(Pattern, Option<Node>, Node)> = Vec::new();
        if self.peek_token == Token::RightBrace {
            self.next_token(); // to '}'
            return Node::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            span: self.span_at_current()
            };
        }

        self.next_token(); // skip '{'
        loop {
            let pattern = self.parse_pattern();
            self.next_token(); // advance past the pattern to '=>' or 'if'

            // RES-159: optional guard — `case <pattern> if <expr> =>`.
            // Evaluated at eval time in the pattern's binding scope;
            // a `false` guard falls through to the next arm.
            let guard = if self.current_token == Token::If {
                self.next_token(); // past `if`
                let g = self.parse_expression(0).unwrap_or(
                    Node::BooleanLiteral { value: true, span: span::Span::default() }
                );
                self.next_token(); // past last token of guard
                Some(g)
            } else {
                None
            };

            if self.current_token != Token::FatArrow {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected '=>' after match pattern, found {}",
                    tok
                ));
                break;
            }
            self.next_token(); // skip '=>'
            let body = self.parse_expression(0).unwrap_or(Node::IntegerLiteral { value: 0, span: span::Span::default() });
            arms.push((pattern, guard, body));
            self.next_token(); // past last token of body
            if self.current_token == Token::Comma {
                self.next_token();
            }
            if self.current_token == Token::RightBrace {
                break;
            }
            if matches!(self.current_token, Token::Eof) {
                self.record_error("Unexpected EOF inside match expression".to_string());
                break;
            }
        }

        Node::Match {
            scrutinee: Box::new(scrutinee),
            arms,
            span: self.span_at_current()
        }
    }

    /// Parse a single match pattern, possibly with `|` alternatives
    /// (RES-160). On exit, `current_token` is the last token of the
    /// last atom (so the caller's `next_token()` advances past the
    /// pattern as a whole).
    fn parse_pattern(&mut self) -> Pattern {
        let first = self.parse_pattern_atom();
        // RES-160: collect `| <pattern>` tails. `|` is
        // `Token::BitOr`; a lone `|` in pattern position is
        // unambiguous since no pattern atom starts with `|`.
        if self.peek_token != Token::BitOr {
            return first;
        }
        let mut branches: Vec<Pattern> = vec![first];
        while self.peek_token == Token::BitOr {
            self.next_token(); // current_token = `|`
            self.next_token(); // past `|` to the next atom
            let next = self.parse_pattern_atom();
            branches.push(next);
        }
        Pattern::Or(branches)
    }

    /// RES-160: parse a single, atomic match pattern (no top-level
    /// `|`). Single-token patterns only for now — structural
    /// patterns (tuples, struct destructure in match) land with
    /// RES-161 and friends.
    fn parse_pattern_atom(&mut self) -> Pattern {
        let tok_span = self.span_at_current();
        match &self.current_token {
            Token::Underscore => Pattern::Wildcard,
            // RES-163: `default` desugars to `_` — pure sugar, no
            // downstream phase sees a distinct variant. Only legal
            // at pattern position; other uses surface as an
            // unexpected-token error since `Token::Default` isn't
            // accepted anywhere else in the grammar.
            Token::Default => Pattern::Wildcard,
            Token::IntLiteral(n) => Pattern::Literal(Node::IntegerLiteral { value: *n, span: tok_span }),
            Token::FloatLiteral(f) => Pattern::Literal(Node::FloatLiteral { value: *f, span: tok_span }),
            Token::StringLiteral(s) => Pattern::Literal(Node::StringLiteral { value: s.clone(), span: tok_span }),
            Token::BoolLiteral(b) => Pattern::Literal(Node::BooleanLiteral { value: *b, span: tok_span }),
            Token::Identifier(name) => Pattern::Identifier(name.clone()),
            other => {
                let tok = other.clone();
                self.record_error(format!(
                    "Unsupported match pattern starting with {:?}",
                    tok
                ));
                Pattern::Wildcard
            }
        }
    }

    /// Parse `.field`. current_token is `.` on entry; on exit current is `field`.
    fn parse_field_access(&mut self, target: Node) -> Option<Node> {
        // RES-085: span covers the `.` at current_token on entry.
        let dot_span = self.span_at_current();
        self.next_token(); // skip '.'
        let field = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!("Expected field name after '.', found {}", tok));
                return Some(target);
            }
        };
        Some(Node::FieldAccess {
            target: Box::new(target),
            field,
            span: dot_span,
        })
    }

    /// Parse `[e1, e2, ...]`. current_token is `[` on entry; on exit
    /// current_token is `]`.
    fn parse_array_literal(&mut self) -> Node {
        // RES-086: capture the `[` token's span before advancing.
        let bracket_span = self.span_at_current();
        let mut items = Vec::new();
        if self.peek_token == Token::RightBracket {
            self.next_token(); // to ]
            return Node::ArrayLiteral { items, span: bracket_span };
        }
        self.next_token(); // skip '['
        if let Some(first) = self.parse_expression(0) {
            // RES-156: if the next token after the first expression
            // is `for`, this is a comprehension, not an array
            // literal. Steal the first expression as the
            // comprehension's result expression and desugar.
            if self.peek_token == Token::For {
                return self.parse_array_comprehension(first, bracket_span);
            }
            items.push(first);
        }
        while self.peek_token == Token::Comma {
            self.next_token(); // to ','
            // Trailing comma before `]` is allowed.
            if self.peek_token == Token::RightBracket {
                break;
            }
            self.next_token(); // skip ','
            if let Some(next) = self.parse_expression(0) {
                items.push(next);
            }
        }
        if self.peek_token != Token::RightBracket {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ']' to close array literal, found {}",
                tok
            ));
        } else {
            self.next_token(); // to ]
        }
        Node::ArrayLiteral { items, span: bracket_span }
    }

    /// RES-156: desugar `[<expr> for <binding> in <iterable> (if
    /// <guard>)?]` into an immediately-invoked fn:
    ///
    /// ```text
    /// (fn() {
    ///   let _r$N = [];
    ///   for <binding> in <iterable> {
    ///     if (<guard>) { _r$N = push(_r$N, <expr>); }
    ///   }
    ///   return _r$N;
    /// })()
    /// ```
    ///
    /// On entry, `current_token` sits at the last token of
    /// `<expr>`; `peek_token == For`. On exit, `current_token` is
    /// the closing `]` so `parse_expression`'s tail handling works
    /// unchanged. `first_expr` is the already-parsed result
    /// expression.
    fn parse_array_comprehension(
        &mut self,
        first_expr: Node,
        bracket_span: span::Span,
    ) -> Node {
        // Advance from the end of <expr> to `for`.
        self.next_token(); // current_token == Token::For
        self.next_token(); // past `for` to the binding identifier

        let binding = match &self.current_token {
            Token::Identifier(n) => n.clone(),
            _ => {
                let tok = self.current_token.clone();
                self.record_error(format!(
                    "Expected binding name after `for` in comprehension, found {}",
                    tok
                ));
                // Best-effort fallback: synthesize a placeholder so
                // parsing proceeds — the user will see the recorded
                // error.
                "_comp_err".to_string()
            }
        };
        self.next_token(); // past binding

        if self.current_token != Token::In {
            let tok = self.current_token.clone();
            self.record_error(format!(
                "Expected `in` after comprehension binding `{}`, found {}",
                binding, tok
            ));
        } else {
            self.next_token(); // past `in`
        }

        let iterable = self
            .parse_expression(0)
            .unwrap_or(Node::ArrayLiteral {
                items: Vec::new(),
                span: span::Span::default(),
            });

        // Optional guard: `if <expr>`.
        let guard = if self.peek_token == Token::If {
            self.next_token(); // current_token == `if`
            self.next_token(); // past `if`
            self.parse_expression(0)
        } else {
            None
        };

        // Expect the closing `]`.
        if self.peek_token != Token::RightBracket {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected `]` to close array comprehension, found {}",
                tok
            ));
        } else {
            self.next_token(); // to `]`
        }

        // ---------- Desugar ----------
        let acc = format!("_r${}", self.comprehension_counter);
        self.comprehension_counter += 1;
        let default = span::Span::default;

        // push(_r, <expr>)
        let push_call = Node::CallExpression {
            function: Box::new(Node::Identifier {
                name: "push".to_string(),
                span: default(),
            }),
            arguments: vec![
                Node::Identifier {
                    name: acc.clone(),
                    span: default(),
                },
                first_expr,
            ],
            span: default(),
        };

        // _r = push(_r, <expr>);
        let push_assign = Node::Assignment {
            name: acc.clone(),
            value: Box::new(push_call),
            span: default(),
        };

        // Loop body: either wrapped in `if <guard> { ... }` or the
        // bare assign. Keep the inner structure a Block either way
        // so `ForInStatement`'s body shape is uniform.
        let inner_block = match guard {
            Some(g) => Node::Block {
                stmts: vec![Node::IfStatement {
                    condition: Box::new(g),
                    consequence: Box::new(Node::Block {
                        stmts: vec![push_assign],
                        span: default(),
                    }),
                    alternative: None,
                    span: default(),
                }],
                span: default(),
            },
            None => Node::Block {
                stmts: vec![push_assign],
                span: default(),
            },
        };

        // for <binding> in <iterable> { body }
        let for_stmt = Node::ForInStatement {
            name: binding,
            iterable: Box::new(iterable),
            body: Box::new(inner_block),
            span: default(),
        };

        // let _r = [];
        let init_let = Node::LetStatement {
            name: acc.clone(),
            value: Box::new(Node::ArrayLiteral {
                items: Vec::new(),
                span: default(),
            }),
            type_annot: None,
            span: default(),
        };

        // return _r;
        let ret_stmt = Node::ReturnStatement {
            value: Some(Box::new(Node::Identifier {
                name: acc,
                span: default(),
            })),
            span: default(),
        };

        // { let _r = []; for ... { ... } return _r; }
        let body_block = Node::Block {
            stmts: vec![init_let, for_stmt, ret_stmt],
            span: default(),
        };

        // fn() { body_block }
        let fn_lit = Node::FunctionLiteral {
            parameters: Vec::new(),
            body: Box::new(body_block),
            requires: Vec::new(),
            ensures: Vec::new(),
            return_type: None,
            span: bracket_span,
        };

        // (fn() { ... })()
        Node::CallExpression {
            function: Box::new(fn_lit),
            arguments: Vec::new(),
            span: bracket_span,
        }
    }

    /// RES-148: parse a map literal — `{k -> v, k2 -> v2, ...}`.
    /// `current_token` is `{` on entry; on exit it is `}`. The
    /// disambiguation against statement-level `{` (blocks) is handled
    /// by `parse_expression` only invoking this when it sees `{` in
    /// expression position.
    ///
    /// Trailing comma is accepted, like the array parser.
    fn parse_map_literal(&mut self) -> Node {
        let brace_span = self.span_at_current();
        let mut entries: Vec<(Node, Node)> = Vec::new();
        // Empty map: `{}`.
        if self.peek_token == Token::RightBrace {
            self.next_token(); // to '}'
            return Node::MapLiteral { entries, span: brace_span };
        }
        self.next_token(); // step past '{'
        // First entry.
        if let Some((k, v)) = self.parse_map_entry() {
            entries.push((k, v));
        }
        while self.peek_token == Token::Comma {
            self.next_token(); // to ','
            if self.peek_token == Token::RightBrace {
                break; // trailing comma
            }
            self.next_token(); // step past ','
            if let Some((k, v)) = self.parse_map_entry() {
                entries.push((k, v));
            }
        }
        if self.peek_token != Token::RightBrace {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected '}}' to close map literal, found {}",
                tok
            ));
        } else {
            self.next_token(); // to '}'
        }
        Node::MapLiteral { entries, span: brace_span }
    }

    /// RES-149: parse a set literal — `#{1, 2, 3}`. `current_token`
    /// is `#{` on entry; on exit it is the closing `}`. Mirrors the
    /// map parser's shape (comma-separated, trailing comma allowed,
    /// empty via `#{}`).
    fn parse_set_literal(&mut self) -> Node {
        let brace_span = self.span_at_current();
        let mut items: Vec<Node> = Vec::new();
        // Empty: `#{}`.
        if self.peek_token == Token::RightBrace {
            self.next_token(); // to `}`
            return Node::SetLiteral { items, span: brace_span };
        }
        self.next_token(); // step past `#{`
        // First item.
        if let Some(n) = self.parse_expression(0) {
            items.push(n);
        }
        while self.peek_token == Token::Comma {
            self.next_token(); // to `,`
            if self.peek_token == Token::RightBrace {
                break; // trailing comma before `}`
            }
            self.next_token(); // step past `,`
            if let Some(n) = self.parse_expression(0) {
                items.push(n);
            }
        }
        if self.peek_token != Token::RightBrace {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected '}}' to close set literal, found {}",
                tok
            ));
        } else {
            self.next_token(); // to `}`
        }
        Node::SetLiteral { items, span: brace_span }
    }

    /// RES-148: parse a single `key -> value` pair. `current_token`
    /// is the first token of the key expression on entry; on exit
    /// it is the last token of the value expression.
    fn parse_map_entry(&mut self) -> Option<(Node, Node)> {
        let key = self.parse_expression(0)?;
        if self.peek_token != Token::Arrow {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected '->' between map key and value, found {}",
                tok
            ));
            return None;
        }
        self.next_token(); // to '->'
        self.next_token(); // step past '->' to value
        let value = self.parse_expression(0)?;
        Some((key, value))
    }

    /// Parse `target[index]`. current_token is `[` on entry; on exit
    /// current_token is `]`.
    fn parse_index_expression(&mut self, target: Node) -> Option<Node> {
        // RES-085: span covers the `[` at current_token on entry.
        let bracket_span = self.span_at_current();
        self.next_token(); // skip '['
        let index = self.parse_expression(0)?;
        if self.peek_token != Token::RightBracket {
            let tok = self.peek_token.clone();
            self.record_error(format!(
                "Expected ']' to close index expression, found {}",
                tok
            ));
            return Some(Node::IndexExpression {
                target: Box::new(target),
                index: Box::new(index),
                span: bracket_span,
            });
        }
        self.next_token(); // to ]
        Some(Node::IndexExpression {
            target: Box::new(target),
            index: Box::new(index),
            span: bracket_span,
        })
    }

    fn current_precedence(&self) -> u8 {
        match &self.current_token {
            Token::Or => 1,
            Token::And => 2,
            Token::BitOr => 3,
            Token::BitXor => 4,
            Token::BitAnd => 5,
            Token::Equal | Token::NotEqual => 6,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 7,
            Token::ShiftLeft | Token::ShiftRight => 8,
            Token::Plus | Token::Minus => 9,
            Token::Multiply | Token::Divide | Token::Modulo => 10,
            Token::LeftParen => 11,
            Token::LeftBracket => 11,
            Token::Dot => 11,
            Token::Question => 12,
            _ => 0,
        }
    }
    
    fn peek_precedence(&self) -> u8 {
        match &self.peek_token {
            Token::Or => 1,
            Token::And => 2,
            Token::BitOr => 3,
            Token::BitXor => 4,
            Token::BitAnd => 5,
            Token::Equal | Token::NotEqual => 6,
            Token::Less | Token::Greater | Token::LessEqual | Token::GreaterEqual => 7,
            Token::ShiftLeft | Token::ShiftRight => 8,
            Token::Plus | Token::Minus => 9,
            Token::Multiply | Token::Divide | Token::Modulo => 10,
            Token::LeftParen => 11,
            Token::LeftBracket => 11,
            Token::Dot => 11,
            Token::Question => 12,
            _ => 0,
        }
    }
}

// Signature for native Rust functions exposed to the interpreter.
type BuiltinFn = fn(&[Value]) -> RResult<Value>;

// Value types for our interpreter
#[derive(Clone)]
enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Function {
        parameters: Vec<(String, String)>,
        body: Box<Node>,
        env: Environment,
        /// RES-035: pre-conditions propagated into the runtime Value so
        /// apply_function can check them. Empty when absent.
        requires: Vec<Node>,
        ensures: Vec<Node>,
        /// Function name — used for better contract-violation messages.
        name: String,
    },
    /// Native function. `name` is the identifier it was registered as,
    /// for diagnostics only.
    Builtin {
        name: &'static str,
        func: BuiltinFn,
    },
    /// RES-032: dynamic array. Mixed types allowed at runtime until a
    /// real type system (G7) can enforce a single element type.
    Array(Vec<Value>),
    /// RES-038: user-defined record. Fields are stored in declaration
    /// order so Display is stable.
    Struct {
        name: String,
        fields: Vec<(String, Value)>,
    },
    /// RES-040: first-class Result type.
    ///
    /// `ok = true` means the payload is the success value.
    /// `ok = false` means the payload is the error (typically a
    /// `Value::String`, but any value is allowed).
    Result {
        ok: bool,
        payload: Box<Value>,
    },
    Return(Box<Value>),
    Void,
    /// RES-148: associative map. Keys are restricted (via `MapKey`) to
    /// the hashable primitives (`Int`, `String`, `Bool`) — anything
    /// else at a key slot is a runtime error. The interpreter lives in
    /// `std`, so we use `HashMap`; the `resilient-runtime` sibling
    /// crate has no `Value::Map` at all and stays no_std-clean.
    ///
    /// Value identity is structural — two maps compare equal when
    /// their (K, V) pair sets match. (Implemented case-by-case in the
    /// few paths that need it; `Value` itself does not derive
    /// `PartialEq`.)
    Map(std::collections::HashMap<MapKey, Value>),
    /// RES-149: unordered set of hashable primitives. Element type
    /// is the same `MapKey` that powers `Value::Map` keys — one
    /// policy, one enforcement site. Iteration order is unspecified
    /// on `std` (hash-based); the sibling no_std runtime would back
    /// with `BTreeSet` for sorted iteration (tracked as a follow-up
    /// when the runtime grows a set value type).
    Set(std::collections::HashSet<MapKey>),
    /// RES-152: raw byte sequence — protocol frames, register maps,
    /// packed on-the-wire layouts. Distinct from `String`: users
    /// bridge via explicit builtins, and the typechecker rejects
    /// passing a `Bytes` where a `String` is expected and vice
    /// versa. No interior mutability — `bytes_slice` returns a new
    /// `Value::Bytes`.
    Bytes(Vec<u8>),
}

/// RES-148: hashable-key restriction for `Value::Map`. Only the three
/// primitives (`Int`, `String`, `Bool`) are permitted; anything else
/// at a key position surfaces a runtime error via `MapKey::from_value`.
/// Derives `Hash + Eq` so `HashMap` works without any custom hasher.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum MapKey {
    Int(i64),
    Str(String),
    Bool(bool),
}

impl MapKey {
    /// Coerce a `Value` into a `MapKey`, returning a runtime error if
    /// the value is not one of the three hashable primitives.
    fn from_value(v: &Value) -> Result<Self, String> {
        match v {
            Value::Int(n) => Ok(MapKey::Int(*n)),
            Value::String(s) => Ok(MapKey::Str(s.clone())),
            Value::Bool(b) => Ok(MapKey::Bool(*b)),
            other => Err(format!(
                "Map key must be Int, String, or Bool; got {}",
                other
            )),
        }
    }

    /// Reverse: `MapKey` → `Value` for round-tripping into the
    /// interpreter's Value plane (used by `map_keys`).
    fn to_value(&self) -> Value {
        match self {
            MapKey::Int(n) => Value::Int(*n),
            MapKey::Str(s) => Value::String(s.clone()),
            MapKey::Bool(b) => Value::Bool(*b),
        }
    }
}

impl std::fmt::Display for MapKey {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            MapKey::Int(n) => write!(f, "{}", n),
            MapKey::Str(s) => write!(f, "\"{}\"", s),
            MapKey::Bool(b) => write!(f, "{}", b),
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "Int({})", i),
            Value::Float(fl) => write!(f, "Float({})", fl),
            Value::String(s) => write!(f, "String({:?})", s),
            Value::Bool(b) => write!(f, "Bool({})", b),
            Value::Function { parameters, .. } => {
                write!(f, "Function({} params)", parameters.len())
            }
            Value::Builtin { name, .. } => write!(f, "Builtin({})", name),
            Value::Array(items) => write!(f, "Array({} items)", items.len()),
            Value::Struct { name, fields } => {
                write!(f, "Struct({}, {} fields)", name, fields.len())
            }
            Value::Result { ok, payload } => {
                write!(f, "{}({:?})", if *ok { "Ok" } else { "Err" }, payload)
            }
            Value::Return(v) => write!(f, "Return({:?})", v),
            Value::Void => write!(f, "Void"),
            Value::Map(m) => write!(f, "Map({} entries)", m.len()),
            Value::Set(s) => write!(f, "Set({} items)", s.len()),
            Value::Bytes(b) => write!(f, "Bytes({} bytes)", b.len()),
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Float(fl) => write!(f, "{}", fl),
            Value::String(s) => write!(f, "\"{}\"", s),
            Value::Bool(b) => write!(f, "{}", b),
            Value::Function { .. } => write!(f, "<function>"),
            Value::Builtin { name, .. } => write!(f, "<builtin {}>", name),
            Value::Array(items) => {
                write!(f, "[")?;
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Struct { name, fields } => {
                write!(f, "{} {{ ", name)?;
                for (i, (fname, fval)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", fname, fval)?;
                }
                write!(f, " }}")
            }
            Value::Result { ok, payload } => {
                write!(f, "{}({})", if *ok { "Ok" } else { "Err" }, payload)
            }
            Value::Return(v) => write!(f, "{}", v),
            Value::Void => write!(f, "void"),
            Value::Map(m) => {
                // RES-148: iterate keys in sorted order so Display is
                // deterministic across runs — HashMap's iteration
                // order would make golden tests flaky otherwise.
                write!(f, "{{")?;
                let mut keys: Vec<&MapKey> = m.keys().collect();
                keys.sort_by(|a, b| match (a, b) {
                    (MapKey::Int(x), MapKey::Int(y)) => x.cmp(y),
                    (MapKey::Str(x), MapKey::Str(y)) => x.cmp(y),
                    (MapKey::Bool(x), MapKey::Bool(y)) => x.cmp(y),
                    // Different key types: Int < Str < Bool for stable tie-break.
                    (MapKey::Int(_), _) => std::cmp::Ordering::Less,
                    (_, MapKey::Int(_)) => std::cmp::Ordering::Greater,
                    (MapKey::Str(_), _) => std::cmp::Ordering::Less,
                    (_, MapKey::Str(_)) => std::cmp::Ordering::Greater,
                });
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{} -> {}", k, m.get(k).expect("key is from map"))?;
                }
                write!(f, "}}")
            }
            Value::Bytes(b) => {
                // RES-152: print as a `b"..."` literal with hex
                // escapes for non-printable bytes and the five
                // named escapes, so Display round-trips through the
                // lexer.
                write!(f, "b\"")?;
                for &byte in b {
                    match byte {
                        b'\\' => write!(f, "\\\\")?,
                        b'"' => write!(f, "\\\"")?,
                        b'\n' => write!(f, "\\n")?,
                        b'\r' => write!(f, "\\r")?,
                        b'\t' => write!(f, "\\t")?,
                        // Printable ASCII (space through `~`)
                        // renders as itself — everything else as
                        // `\xNN`. Matches the ticket's
                        // "hex escapes required for non-printable
                        // bytes" guidance.
                        0x20..=0x7E => write!(f, "{}", byte as char)?,
                        _ => write!(f, "\\x{:02x}", byte)?,
                    }
                }
                write!(f, "\"")
            }
            Value::Set(s) => {
                // RES-149: mirror Map's Display — sort keys for
                // deterministic output despite the underlying
                // HashSet having arbitrary iteration order.
                write!(f, "#{{")?;
                let mut items: Vec<&MapKey> = s.iter().collect();
                items.sort_by(|a, b| match (a, b) {
                    (MapKey::Int(x), MapKey::Int(y)) => x.cmp(y),
                    (MapKey::Str(x), MapKey::Str(y)) => x.cmp(y),
                    (MapKey::Bool(x), MapKey::Bool(y)) => x.cmp(y),
                    (MapKey::Int(_), _) => std::cmp::Ordering::Less,
                    (_, MapKey::Int(_)) => std::cmp::Ordering::Greater,
                    (MapKey::Str(_), _) => std::cmp::Ordering::Less,
                    (_, MapKey::Str(_)) => std::cmp::Ordering::Greater,
                });
                for (i, k) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", k)?;
                }
                write!(f, "}}")
            }
        }
    }
}

// Result type for handling errors in our language
type RResult<T> = Result<T, String>;

// Environment for storing variables.
//
// RES-050: Environment is a thin wrapper around `Rc<RefCell<EnvFrame>>`.
// Cloning is just an Rc bump (atomic increment), not a deep copy of
// the HashMap. This is the key to:
//
//   - Recursion working without the apply_function self-bind hack
//     (a function captured before its name is rebound now sees the
//     same RefCell that gets the rebind).
//
//   - Closures with shared mutation: an inner fn that captures `n`
//     and mutates it actually mutates the SAME slot the outer fn
//     reads, instead of a snapshot.
//
//   - The fn-call hot path no longer pays for HashMap clone on
//     every call. fib(25) goes from ~2.9s to <100ms (see
//     benchmarks/RESULTS.md after this lands).
//
// All mutation methods take &self because the RefCell handles
// interior mutability.
#[derive(Debug, Clone)]
struct Environment {
    inner: Rc<RefCell<EnvFrame>>,
}

#[derive(Debug)]
struct EnvFrame {
    store: HashMap<String, Value>,
    outer: Option<Environment>,
}

impl Environment {
    fn new() -> Self {
        Environment {
            inner: Rc::new(RefCell::new(EnvFrame {
                store: HashMap::new(),
                outer: None,
            })),
        }
    }

    fn new_enclosed(outer: Environment) -> Self {
        Environment {
            inner: Rc::new(RefCell::new(EnvFrame {
                store: HashMap::new(),
                outer: Some(outer),
            })),
        }
    }

    fn get(&self, name: &str) -> Option<Value> {
        // Take the borrow, look up locally, then if absent walk into
        // outer. Cloning the outer Environment for the recursive call
        // is cheap (Rc bump).
        let frame = self.inner.borrow();
        if let Some(v) = frame.store.get(name) {
            return Some(v.clone());
        }
        let outer = frame.outer.clone();
        drop(frame);
        outer.and_then(|o| o.get(name))
    }

    fn set(&self, name: String, value: Value) {
        self.inner.borrow_mut().store.insert(name, value);
    }

    /// Update `name` in the frame where it was first defined. Returns
    /// `true` if the name was found and updated, `false` if it doesn't
    /// exist anywhere in the chain.
    fn reassign(&self, name: &str, value: Value) -> bool {
        let mut frame = self.inner.borrow_mut();
        if frame.store.contains_key(name) {
            frame.store.insert(name.to_string(), value);
            return true;
        }
        // Drop the borrow before recursing so the outer's borrow_mut
        // doesn't collide if the chain happens to alias.
        let outer = frame.outer.clone();
        drop(frame);
        match outer {
            Some(o) => o.reassign(name, value),
            None => false,
        }
    }

    /// Whole-chain deep copy. Allocates fresh RefCells for every frame
    /// and copies each HashMap by value. Function values inside the
    /// store are cloned shallowly (their captured envs remain Rc-shared
    /// with the original) — that's correct because fn definitions
    /// don't change during a live-block retry; only variable values do.
    ///
    /// This restores the pre-RES-050 semantics that `live { }` blocks
    /// depend on: snapshot the env at entry, restore on every retry so
    /// each attempt sees the same initial state.
    fn deep_clone(&self) -> Environment {
        let frame = self.inner.borrow();
        Environment {
            inner: Rc::new(RefCell::new(EnvFrame {
                store: frame.store.clone(),
                outer: frame.outer.as_ref().map(|o| o.deep_clone()),
            })),
        }
    }
}

/// Walk a `FieldAssignment` target tree, collecting the chain of field
/// names. Returns (root_identifier, [field1, field2, ...]). If the root
/// isn't an identifier, the first return is None.
fn flatten_field_target(target: &Node, last_field: &str) -> (Option<String>, Vec<String>) {
    let mut path = vec![last_field.to_string()];
    let mut node = target;
    loop {
        match node {
            Node::Identifier { name, .. } => return (Some(name.clone()), {
                path.reverse();
                path
            }),
            Node::FieldAccess { target: t, field, .. } => {
                path.push(field.clone());
                node = t;
            }
            _ => return (None, Vec::new()),
        }
    }
}

/// Given a root struct value, set the field chain to `new_val` and
/// return the updated root. Errors if any intermediate is not a struct
/// or a field is absent.
fn set_nested_field(root: Value, path: &[String], new_val: Value) -> RResult<Value> {
    if path.is_empty() {
        return Ok(new_val);
    }
    match root {
        Value::Struct { name, mut fields } => {
            let head = &path[0];
            let tail = &path[1..];
            let idx = fields.iter().position(|(n, _)| n == head).ok_or_else(|| {
                format!("Struct {} has no field '{}'", name, head)
            })?;
            let old = std::mem::replace(&mut fields[idx].1, Value::Void);
            let updated = set_nested_field(old, tail, new_val)?;
            fields[idx].1 = updated;
            Ok(Value::Struct { name, fields })
        }
        other => Err(format!(
            "Cannot assign field on non-struct value {:?}",
            other
        )),
    }
}

/// Human-readable rendering of a contract clause for the error message.
/// Deliberately lossy: we just want the user to recognize which clause
/// fired, not reconstruct the full AST.
fn format_contract_expr(node: &Node) -> String {
    match node {
        Node::Identifier { name, .. } => name.clone(),
        Node::IntegerLiteral { value, .. } => value.to_string(),
        Node::FloatLiteral { value, .. } => value.to_string(),
        Node::StringLiteral { value, .. } => format!("{:?}", value),
        Node::BooleanLiteral { value, .. } => value.to_string(),
        Node::PrefixExpression { operator, right, .. } => {
            format!("{}{}", operator, format_contract_expr(right))
        }
        Node::InfixExpression { left, operator, right, .. } => {
            format!(
                "{} {} {}",
                format_contract_expr(left),
                operator,
                format_contract_expr(right)
            )
        }
        Node::CallExpression { function, arguments, .. } => {
            let args: Vec<String> = arguments.iter().map(format_contract_expr).collect();
            format!("{}({})", format_contract_expr(function), args.join(", "))
        }
        Node::IndexExpression { target, index, .. } => {
            format!(
                "{}[{}]",
                format_contract_expr(target),
                format_contract_expr(index)
            )
        }
        _ => "<expr>".to_string(),
    }
}

/// Textual form of a value for string concatenation (`+` with at least one
/// string operand). Returns `None` for values that should NOT be implicitly
/// coerced (functions, builtins, void, returns). Strings come back as their
/// raw contents — *without* the surrounding quotes that `Display` adds.
fn stringify_for_concat(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Float(f) => Some(f.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

// ---------- Builtins ----------
//
// Native functions registered into every Interpreter's top-level
// environment. Keep this list small and predictable — it is the
// language's minimal stdlib until a proper module system arrives.

// RES-050: Environment methods take &self; this signature could now be
// `&Environment`, but `&mut` is harmless and signals "we're populating".
fn register_builtins(env: &mut Environment) {
    for (name, func) in BUILTINS {
        env.set(
            (*name).to_string(),
            Value::Builtin {
                name,
                func: *func,
            },
        );
    }
}

/// Canonical list of every native function visible in a fresh
/// Resilient program.
const BUILTINS: &[(&str, BuiltinFn)] = &[
    ("println", builtin_println),
    ("print", builtin_print),
    // RES-144: single-line stdin read. std-only.
    ("input", builtin_input),
    ("abs", builtin_abs),
    ("min", builtin_min),
    ("max", builtin_max),
    // RES-130: explicit int ↔ float conversions.
    ("to_float", builtin_to_float),
    ("to_int", builtin_to_int),
    // RES-138: read the current retry count inside a live block.
    ("live_retries", builtin_live_retries),
    // RES-141: process-wide live-block telemetry.
    ("live_total_retries", builtin_live_total_retries),
    ("live_total_exhaustions", builtin_live_total_exhaustions),
    ("sqrt", builtin_sqrt),
    ("pow", builtin_pow),
    ("floor", builtin_floor),
    ("ceil", builtin_ceil),
    // RES-146: transcendentals. std-only; float-in/float-out per
    // RES-130's no-implicit-coercion policy.
    ("sin", builtin_sin),
    ("cos", builtin_cos),
    ("tan", builtin_tan),
    ("ln", builtin_ln),
    ("log", builtin_log),
    ("exp", builtin_exp),
    // RES-147: monotonic ms clock, std-only.
    ("clock_ms", builtin_clock_ms),
    // RES-150: seedable SplitMix64 random builtins. std-only.
    ("random_int", builtin_random_int),
    ("random_float", builtin_random_float),
    ("len", builtin_len),
    ("push", builtin_push),
    ("pop", builtin_pop),
    ("slice", builtin_slice),
    ("split", builtin_split),
    ("trim", builtin_trim),
    ("contains", builtin_contains),
    ("to_upper", builtin_to_upper),
    ("to_lower", builtin_to_lower),
    // RES-145: string manipulation expansion.
    ("replace", builtin_replace),
    ("format", builtin_format),
    ("Ok", builtin_ok),
    ("Err", builtin_err),
    ("is_ok", builtin_is_ok),
    ("is_err", builtin_is_err),
    ("unwrap", builtin_unwrap),
    ("unwrap_err", builtin_unwrap_err),
    // RES-143: file I/O. Std-only; the `resilient-runtime` crate has
    // no builtins table and stays no_std-clean.
    ("file_read", builtin_file_read),
    ("file_write", builtin_file_write),
    // RES-151: read-only env-var accessor, std-only.
    ("env", builtin_env),
    // RES-148: Map builtins.
    ("map_new", builtin_map_new),
    ("map_insert", builtin_map_insert),
    ("map_get", builtin_map_get),
    ("map_remove", builtin_map_remove),
    ("map_keys", builtin_map_keys),
    ("map_len", builtin_map_len),
    // RES-149: Set builtins.
    ("set_new", builtin_set_new),
    ("set_insert", builtin_set_insert),
    ("set_remove", builtin_set_remove),
    ("set_has", builtin_set_has),
    ("set_len", builtin_set_len),
    ("set_items", builtin_set_items),
    // RES-152: Bytes builtins.
    ("bytes_len", builtin_bytes_len),
    ("bytes_slice", builtin_bytes_slice),
    ("byte_at", builtin_byte_at),
];

/// Print the single argument followed by a newline and return `Void`.
///
/// Strings print without surrounding quotes (so `println("hi")` writes
/// `hi`, not `"hi"`). Other values print via their `Display` impl.
fn builtin_println(args: &[Value]) -> RResult<Value> {
    match args {
        [] => {
            println!();
            Ok(Value::Void)
        }
        [single] => {
            match single {
                Value::String(s) => println!("{}", s),
                other => println!("{}", other),
            }
            Ok(Value::Void)
        }
        many => Err(format!(
            "println expects 0 or 1 argument, got {}",
            many.len()
        )),
    }
}

/// `print(x)` — like println but without the trailing newline. Useful
/// for building a line from multiple values or for prompt-style output.
fn builtin_print(args: &[Value]) -> RResult<Value> {
    use std::io::Write as _;
    match args {
        [] => {
            // No-op with flush so partial-line state is consistent.
            let _ = std::io::stdout().flush();
            Ok(Value::Void)
        }
        [single] => {
            match single {
                Value::String(s) => print!("{}", s),
                other => print!("{}", other),
            }
            let _ = std::io::stdout().flush();
            Ok(Value::Void)
        }
        many => Err(format!("print expects 0 or 1 argument, got {}", many.len())),
    }
}

/// RES-144: `input(prompt: String) -> String` — read one line from
/// stdin, returning everything up to (but not including) the first
/// `\n`. Any trailing `\r` is also stripped so Windows-style line
/// endings round-trip cleanly. An empty `prompt` skips the prompt
/// print; otherwise the prompt is written to stdout and flushed
/// before the read begins so interactive shells see it immediately.
///
/// EOF before any bytes are read returns `""` (not an error) — this
/// lets idiomatic `while input("> ") != "quit" { ... }` loops exit
/// on ctrl-D without tripping an exception.
///
/// std-only: the no_std `resilient-runtime` crate has no builtins
/// table and stays embedded-clean.
fn builtin_input(args: &[Value]) -> RResult<Value> {
    let prompt = match args {
        [Value::String(s)] => s.clone(),
        [other] => {
            return Err(format!(
                "input: expected String prompt, got {}",
                other
            ))
        }
        many => {
            return Err(format!(
                "input: expected 1 argument (prompt), got {}",
                many.len()
            ))
        }
    };
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    do_input(&mut lock, &prompt)
}

/// Core of `input()` factored out for unit testing — generic over
/// `BufRead` so tests can drive it with a `std::io::Cursor` without
/// blocking on real stdin (per the RES-144 acceptance criterion
/// "stubbed stdin via `std::io::Cursor` injected through a small
/// trait"; here `BufRead` is that trait).
fn do_input<R: std::io::BufRead>(reader: &mut R, prompt: &str) -> RResult<Value> {
    use std::io::Write as _;
    if !prompt.is_empty() {
        print!("{}", prompt);
        // Flush so a prompt without a trailing newline appears
        // before the reader blocks on input.
        let _ = std::io::stdout().flush();
    }
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => Ok(Value::String(String::new())), // EOF
        Ok(_) => {
            // Strip the trailing `\n` (always present on non-EOF)
            // and an optional `\r` for CRLF line endings.
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            Ok(Value::String(line))
        }
        Err(e) => Err(format!("input: stdin read failed: {}", e)),
    }
}

/// `sqrt(x)` — square root, float-returning. Int arg coerced to f64.
fn builtin_sqrt(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float((*i as f64).sqrt())),
        [Value::Float(f)] => Ok(Value::Float(f.sqrt())),
        [other] => Err(format!("sqrt: expected numeric, got {}", other)),
        _ => Err(format!("sqrt: expected 1 argument, got {}", args.len())),
    }
}

/// `pow(base, exp)` — base^exp.
///
/// RES-055: type-preserving. `pow(int, int)` returns `Int` via
/// checked exponentiation (overflow is a clean error, not a panic).
/// Negative integer exponents are a runtime error since `n^-k` is
/// generally not an integer. Mixed int↔float or any float arg keeps
/// the original float behavior.
fn builtin_pow(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(base), Value::Int(exp)] => {
            // Negative exponent isn't representable as Int.
            let exp_u32: u32 = (*exp).try_into().map_err(|_| {
                format!("pow: negative exponent {} undefined for int base", exp)
            })?;
            base.checked_pow(exp_u32).map(Value::Int).ok_or_else(|| {
                format!("pow: integer overflow ({} ^ {})", base, exp)
            })
        }
        [a, b] => {
            let to_f = |v: &Value| match v {
                Value::Int(i) => Some(*i as f64),
                Value::Float(f) => Some(*f),
                _ => None,
            };
            let (Some(base), Some(exp)) = (to_f(a), to_f(b)) else {
                return Err(format!("pow: expected numeric args, got {:?} and {:?}", a, b));
            };
            Ok(Value::Float(base.powf(exp)))
        }
        _ => Err(format!("pow: expected 2 arguments, got {}", args.len())),
    }
}

/// `floor(x)` — round toward negative infinity.
///
/// RES-055: type-preserving. `floor(int)` is the identity — the input
/// is already an integer, no point demoting it to f64. `floor(float)`
/// keeps the original float-returning semantics.
fn builtin_floor(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Int(*i)),
        [Value::Float(f)] => Ok(Value::Float(f.floor())),
        [other] => Err(format!("floor: expected numeric, got {}", other)),
        _ => Err(format!("floor: expected 1 argument, got {}", args.len())),
    }
}

/// `ceil(x)` — round toward positive infinity.
///
/// RES-055: type-preserving. Same logic as floor — `ceil(int)` is the
/// identity, `ceil(float)` returns float.
fn builtin_ceil(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Int(*i)),
        [Value::Float(f)] => Ok(Value::Float(f.ceil())),
        [other] => Err(format!("ceil: expected numeric, got {}", other)),
        _ => Err(format!("ceil: expected 1 argument, got {}", args.len())),
    }
}

// RES-146: transcendental math builtins. Float-only per RES-130
// (no implicit int↔float coercion — users who want `sin(5)` write
// `sin(to_float(5))`). NaN / ±∞ propagate via the underlying `f64`
// methods; we deliberately don't special-case them. std-only for
// now — the no_std runtime will use `libm` in a follow-up ticket.

/// RES-146: `sin(x: Float) -> Float`. Argument is in radians.
fn builtin_sin(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(f)] => Ok(Value::Float(f.sin())),
        [other] => Err(format!(
            "sin: expected Float, got {} — call `to_float(x)` to widen an Int",
            other
        )),
        _ => Err(format!("sin: expected 1 argument, got {}", args.len())),
    }
}

/// RES-146: `cos(x: Float) -> Float`. Radians.
fn builtin_cos(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(f)] => Ok(Value::Float(f.cos())),
        [other] => Err(format!(
            "cos: expected Float, got {} — call `to_float(x)` to widen an Int",
            other
        )),
        _ => Err(format!("cos: expected 1 argument, got {}", args.len())),
    }
}

/// RES-146: `tan(x: Float) -> Float`. Radians. Near ±π/2 the result
/// tends to ±∞; `f64::tan` returns a very large float, not NaN.
fn builtin_tan(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(f)] => Ok(Value::Float(f.tan())),
        [other] => Err(format!(
            "tan: expected Float, got {} — call `to_float(x)` to widen an Int",
            other
        )),
        _ => Err(format!("tan: expected 1 argument, got {}", args.len())),
    }
}

/// RES-146: `ln(x: Float) -> Float` — natural logarithm (base e).
/// `ln(x)` where `x <= 0` is a runtime error. `ln(0)` in f64
/// would return `-inf`; the ticket doesn't specifically require
/// rejecting it, but "Runtime error on non-positive args" is the
/// parallel pattern from `log(base, x)` and treating both
/// consistently is the saner API.
fn builtin_ln(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(f)] => {
            if *f <= 0.0 {
                return Err(format!(
                    "ln: argument must be > 0, got {}",
                    f
                ));
            }
            Ok(Value::Float(f.ln()))
        }
        [other] => Err(format!(
            "ln: expected Float, got {} — call `to_float(x)` to widen an Int",
            other
        )),
        _ => Err(format!("ln: expected 1 argument, got {}", args.len())),
    }
}

/// RES-146: `log(base: Float, x: Float) -> Float` — logarithm of
/// `x` in base `base`. Argument order is base-first to match the
/// English phrasing "log base 2 of 8 is 3"; note Rust's
/// `f64::log(base)` puts base second — the ticket's Notes flag
/// this on purpose.
///
/// Runtime error on `base <= 0`, `base == 1`, or `x <= 0` (per the
/// ticket's "Runtime error on non-positive args or base == 1").
fn builtin_log(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(base), Value::Float(x)] => {
            if *base <= 0.0 {
                return Err(format!(
                    "log: base must be > 0, got {}",
                    base
                ));
            }
            if (*base - 1.0).abs() < f64::EPSILON {
                return Err(
                    "log: base must not be 1 (log_1(x) is undefined)"
                        .to_string(),
                );
            }
            if *x <= 0.0 {
                return Err(format!(
                    "log: value must be > 0, got {}",
                    x
                ));
            }
            Ok(Value::Float(x.log(*base)))
        }
        [a, b] => Err(format!(
            "log: expected (Float, Float), got ({:?}, {:?}) — argument order is (base, value); widen Ints via `to_float`",
            a, b
        )),
        _ => Err(format!(
            "log: expected 2 arguments (base, value), got {}",
            args.len()
        )),
    }
}

/// RES-146: `exp(x: Float) -> Float` — e^x. Overflow at large x
/// produces `+inf` per `f64::exp` — we let it propagate (the
/// ticket says "No special-casing NaN / inf; they propagate").
fn builtin_exp(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Float(f)] => Ok(Value::Float(f.exp())),
        [other] => Err(format!(
            "exp: expected Float, got {} — call `to_float(x)` to widen an Int",
            other
        )),
        _ => Err(format!("exp: expected 1 argument, got {}", args.len())),
    }
}

/// RES-147: process-lifetime monotonic epoch. Lazily captured on
/// the first `clock_ms()` call via `OnceLock` — subsequent calls
/// pay only the atomic-load cost plus an `Instant::now()` sample.
/// The epoch is deliberately unspecified and unobservable except
/// through `clock_ms()`: users get deltas, not absolute times.
static CLOCK_EPOCH: std::sync::OnceLock<std::time::Instant> =
    std::sync::OnceLock::new();

/// RES-150: SplitMix64 — tiny, deterministic, dependency-free PRNG.
///
/// The state is a process-wide `AtomicU64`. `next_u64()` atomically
/// advances the state by the SplitMix64 constant and returns the
/// finalized mix. Relaxed ordering is fine: RNG is a consumer of
/// monotonically-updating state, not a synchronization primitive.
///
/// The seed is set once — either by `--seed <N>` from the CLI, or
/// by `seed_rng_from_clock()` early in `main()`. If nothing seeds
/// it, the first call initializes to `0xdead_beef_cafe_f00d` as a
/// visibly-synthetic default. **This is not cryptographic** — do
/// not use for key material; see README for guidance.
static RNG_STATE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0xdead_beef_cafe_f00d);

/// Remember the seed that was committed so `main` can echo it to
/// stderr ("seed=<N>") per the ticket's reproducibility guarantee.
/// Only written from main at startup; readers sample once.
static RNG_SEED_USED: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0xdead_beef_cafe_f00d);

/// RES-150: install `seed` into the RNG state and remember it for
/// the `seed=<N>` banner. Safe to call at most once from `main`.
fn seed_rng(seed: u64) {
    use std::sync::atomic::Ordering;
    RNG_STATE.store(seed, Ordering::Relaxed);
    RNG_SEED_USED.store(seed, Ordering::Relaxed);
}

/// RES-150: when the CLI didn't pass `--seed`, seed from the
/// monotonic ms clock so repeat runs differ. The seed is logged
/// so the user can pin it via `--seed <N>` on the next run.
fn seed_rng_from_clock() -> u64 {
    let epoch = CLOCK_EPOCH.get_or_init(std::time::Instant::now);
    let ns = std::time::Instant::now()
        .duration_since(*epoch)
        .as_nanos() as u64;
    // XOR with a process-id and a fixed constant so two processes
    // launched at the "same" epoch still differ.
    let pid = std::process::id() as u64;
    let seed = ns
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(pid.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        // Avoid zero — SplitMix64 with a zero state produces a
        // well-defined but uninteresting stream; salt by the
        // ticket's mention of `clock_ms()` so the "default" is
        // legible.
        | 1;
    seed_rng(seed);
    seed
}

/// RES-150: SplitMix64 step. See https://prng.di.unimi.it/splitmix64.c
/// — the three-constant finalizer is the canonical mix.
fn splitmix64_next() -> u64 {
    use std::sync::atomic::Ordering;
    // Advance atomically so concurrent callers each get a distinct
    // pre-finalizer state. Using `fetch_add` with a wrapping
    // constant keeps the generator stateless across threads
    // without a mutex.
    let pre = RNG_STATE.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    let z = pre.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// RES-150: `random_int(lo, hi) -> Int` — half-open `[lo, hi)`.
/// Biased-but-tiny approach: a 64-bit sample mod (hi - lo). Good
/// enough for sim / tests; not uniform over astronomical ranges.
/// Users needing uniformity at full i64 width can roll their own
/// over `random_float() * (hi - lo)` and `to_int`.
fn builtin_random_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(lo), Value::Int(hi)] => {
            if hi <= lo {
                return Err(format!(
                    "random_int: hi must be > lo ({} <= {})",
                    hi, lo
                ));
            }
            let span = (*hi - *lo) as u64;
            let r = splitmix64_next() % span;
            Ok(Value::Int((*lo).wrapping_add(r as i64)))
        }
        [a, b] => Err(format!(
            "random_int: expected (Int, Int), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!(
            "random_int: expected 2 arguments (lo, hi), got {}",
            args.len()
        )),
    }
}

/// RES-150: `random_float() -> Float` — uniform in `[0.0, 1.0)`.
/// The 53-bit conversion (top 53 bits of a u64, divided by 2^53)
/// is the standard trick — gives every representable double in
/// the range a chance proportional to its mantissa density.
fn builtin_random_float(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "random_float: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let x = splitmix64_next();
    // Top 53 bits → f64 in [0, 1).
    let mantissa = x >> 11;
    let f = (mantissa as f64) / ((1u64 << 53) as f64);
    Ok(Value::Float(f))
}

/// RES-147: `clock_ms() -> Int` — milliseconds since a per-process
/// monotonic epoch. Monotonic: `clock_ms()` observed twice in a
/// program never returns a decreasing pair (Rust's
/// `Instant::duration_since` is saturating, and the epoch is frozen
/// at first-call time).
///
/// Returns `Int` (i64). `u128::as_millis` is clamped to `i64::MAX`
/// on the astronomical chance a process runs for ~290 million
/// years, rather than truncating silently.
///
/// std-only: the no_std runtime has no stdlib clock; an
/// `embedded-time` wiring follows in a separate G16 ticket.
fn builtin_clock_ms(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "clock_ms: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let epoch = CLOCK_EPOCH.get_or_init(std::time::Instant::now);
    let ms = std::time::Instant::now().duration_since(*epoch).as_millis();
    // `as_millis` returns u128; clamp to i64::MAX on overflow so
    // long-running processes don't wrap or panic.
    let clamped: i64 = if ms > i64::MAX as u128 {
        i64::MAX
    } else {
        ms as i64
    };
    Ok(Value::Int(clamped))
}

/// `Ok(v)` — wrap a success value as a Result.
fn builtin_ok(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Result {
            ok: true,
            payload: Box::new(v.clone()),
        }),
        _ => Err(format!("Ok: expected 1 argument, got {}", args.len())),
    }
}

/// `Err(e)` — wrap a failure value as a Result.
fn builtin_err(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Result {
            ok: false,
            payload: Box::new(v.clone()),
        }),
        _ => Err(format!("Err: expected 1 argument, got {}", args.len())),
    }
}

/// `is_ok(r)` — true iff `r` is an Ok-tagged Result.
fn builtin_is_ok(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok, .. }] => Ok(Value::Bool(*ok)),
        [other] => Err(format!("is_ok: expected Result, got {}", other)),
        _ => Err(format!("is_ok: expected 1 argument, got {}", args.len())),
    }
}

/// `is_err(r)` — true iff `r` is an Err-tagged Result.
fn builtin_is_err(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok, .. }] => Ok(Value::Bool(!ok)),
        [other] => Err(format!("is_err: expected Result, got {}", other)),
        _ => Err(format!("is_err: expected 1 argument, got {}", args.len())),
    }
}

/// `unwrap(r)` — return the Ok payload or error at runtime.
fn builtin_unwrap(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok: true, payload }] => Ok((**payload).clone()),
        [Value::Result { ok: false, payload }] => {
            Err(format!("unwrap called on Err({})", payload))
        }
        [other] => Err(format!("unwrap: expected Result, got {}", other)),
        _ => Err(format!("unwrap: expected 1 argument, got {}", args.len())),
    }
}

/// `unwrap_err(r)` — return the Err payload or error at runtime.
fn builtin_unwrap_err(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Result { ok: false, payload }] => Ok((**payload).clone()),
        [Value::Result { ok: true, payload }] => {
            Err(format!("unwrap_err called on Ok({})", payload))
        }
        [other] => Err(format!("unwrap_err: expected Result, got {}", other)),
        _ => Err(format!(
            "unwrap_err: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `split(s, sep)` — split `s` at every occurrence of `sep`, returning
/// an array of pieces. Empty `sep` splits into Unicode scalars.
fn builtin_split(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(sep)] => {
            let parts: Vec<Value> = if sep.is_empty() {
                s.chars().map(|c| Value::String(c.to_string())).collect()
            } else {
                s.split(sep.as_str()).map(|p| Value::String(p.to_string())).collect()
            };
            Ok(Value::Array(parts))
        }
        [a, b] => Err(format!(
            "split: expected (string, string), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!("split: expected 2 arguments, got {}", args.len())),
    }
}

/// `trim(s)` — strip leading and trailing ASCII whitespace.
fn builtin_trim(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.trim().to_string())),
        [other] => Err(format!("trim: expected string, got {}", other)),
        _ => Err(format!("trim: expected 1 argument, got {}", args.len())),
    }
}

/// `contains(haystack, needle)` — substring test.
fn builtin_contains(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(h), Value::String(n)] => Ok(Value::Bool(h.contains(n.as_str()))),
        [a, b] => Err(format!(
            "contains: expected (string, string), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!("contains: expected 2 arguments, got {}", args.len())),
    }
}

/// `to_upper(s)` — **ASCII-only** uppercase. Bytes in `a..=z` are
/// mapped to `A..=Z`; every other byte (including all non-ASCII
/// characters) passes through unchanged. Chosen over Unicode
/// `to_uppercase` to avoid locale surprises (e.g. Turkish dotted-i)
/// in safety-critical contexts — predictable beats "right for this
/// locale" when logging or verifying. Per RES-145.
fn builtin_to_upper(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.to_ascii_uppercase())),
        [other] => Err(format!("to_upper: expected string, got {}", other)),
        _ => Err(format!("to_upper: expected 1 argument, got {}", args.len())),
    }
}

/// `to_lower(s)` — **ASCII-only** lowercase; see `to_upper` for the
/// rationale. Per RES-145.
fn builtin_to_lower(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::String(s.to_ascii_lowercase())),
        [other] => Err(format!("to_lower: expected string, got {}", other)),
        _ => Err(format!("to_lower: expected 1 argument, got {}", args.len())),
    }
}

/// RES-145: `replace(s, from, to)` — returns a new string with every
/// non-overlapping, left-to-right occurrence of `from` in `s`
/// replaced by `to`. `from == ""` is a hard error (matches Rust's
/// `str::replace` behaviour, which would splice `to` between every
/// character — almost always a bug). The input string is not
/// mutated.
fn builtin_replace(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s), Value::String(from), Value::String(to)] => {
            if from.is_empty() {
                return Err(
                    "replace: `from` must be non-empty".to_string(),
                );
            }
            Ok(Value::String(s.replace(from.as_str(), to)))
        }
        [a, b, c] => Err(format!(
            "replace: expected (string, string, string), got ({:?}, {:?}, {:?})",
            a, b, c
        )),
        _ => Err(format!(
            "replace: expected 3 arguments, got {}",
            args.len()
        )),
    }
}

/// RES-145: `format(fmt, args)` — interpolate an array of values into
/// a format string. Grammar:
///
/// - `{}` consumes the next argument, left-to-right. The value is
///   rendered via its runtime `Display` impl (so strings print
///   unquoted, like `println`).
/// - `{{` / `}}` escape to a literal `{` / `}`.
/// - Any other use of `{` / `}` is a runtime error: unmatched open,
///   unmatched close, or a non-empty specifier like `{:width}`
///   (deliberately out of scope — this is not printf; see the
///   ticket's Notes).
/// - Mismatched arg count (fewer args than `{}` placeholders, or
///   leftover args) is a runtime error.
fn builtin_format(args: &[Value]) -> RResult<Value> {
    let (fmt, pool) = match args {
        [Value::String(f), Value::Array(a)] => (f, a),
        [a, b] => {
            return Err(format!(
                "format: expected (string, array), got ({:?}, {:?})",
                a, b
            ))
        }
        many => {
            return Err(format!(
                "format: expected 2 arguments (fmt, args), got {}",
                many.len()
            ))
        }
    };

    let mut out = String::with_capacity(fmt.len());
    let mut idx: usize = 0;
    let bytes = fmt.as_bytes();
    let mut i: usize = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                // `{{` → literal `{`.
                if i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    out.push('{');
                    i += 2;
                    continue;
                }
                // `{}` → consume next arg.
                if i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                    let v = pool.get(idx).ok_or_else(|| {
                        format!(
                            "format: not enough arguments — placeholder #{} has no value (got {} total)",
                            idx + 1,
                            pool.len()
                        )
                    })?;
                    match v {
                        Value::String(s) => out.push_str(s),
                        other => out.push_str(&format!("{}", other)),
                    }
                    idx += 1;
                    i += 2;
                    continue;
                }
                // Anything else after `{` is rejected — either an
                // unmatched `{` or a specifier like `{:04}` which
                // the MVP deliberately doesn't parse.
                return Err(format!(
                    "format: unexpected `{{` at byte {} — only `{{}}` (placeholder) and `{{{{` (escaped brace) are supported",
                    i
                ));
            }
            b'}' => {
                // `}}` → literal `}`.
                if i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                    out.push('}');
                    i += 2;
                    continue;
                }
                return Err(format!(
                    "format: unmatched `}}` at byte {} — use `}}}}` to embed a literal `}}`",
                    i
                ));
            }
            _ => {
                // Copy one UTF-8 scalar at a time using the string
                // slice: `char_indices` would do this, but we're
                // already walking bytes. Find the next char boundary.
                let rest = &fmt[i..];
                let ch = rest.chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }

    if idx < pool.len() {
        return Err(format!(
            "format: too many arguments — {} placeholder(s) consumed, {} leftover",
            idx,
            pool.len() - idx
        ));
    }

    Ok(Value::String(out))
}

/// `push(arr, x)` — returns a new array with `x` appended. The input
/// array is not mutated (pass-by-value semantics).
fn builtin_push(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), x] => {
            let mut out = items.clone();
            out.push(x.clone());
            Ok(Value::Array(out))
        }
        [other, _] => Err(format!("push: expected array as first arg, got {}", other)),
        _ => Err(format!("push: expected 2 arguments, got {}", args.len())),
    }
}

/// `pop(arr)` — returns a new array without the last element. Errors on empty.
fn builtin_pop(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items)] => {
            if items.is_empty() {
                Err("pop: cannot pop from an empty array".to_string())
            } else {
                let mut out = items.clone();
                out.pop();
                Ok(Value::Array(out))
            }
        }
        [other] => Err(format!("pop: expected array, got {}", other)),
        _ => Err(format!("pop: expected 1 argument, got {}", args.len())),
    }
}

/// `slice(arr, start, end)` — half-open range `[start, end)`, returning a new array.
fn builtin_slice(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(items), Value::Int(start), Value::Int(end)] => {
            let len = items.len() as i64;
            if *start < 0 || *end < 0 || *start > len || *end > len || *start > *end {
                return Err(format!(
                    "slice: range [{}, {}) is invalid for array of length {}",
                    start, end, len
                ));
            }
            let s = *start as usize;
            let e = *end as usize;
            Ok(Value::Array(items[s..e].to_vec()))
        }
        [a, b, c] => Err(format!(
            "slice: expected (array, int, int), got ({:?}, {:?}, {:?})",
            a, b, c
        )),
        _ => Err(format!("slice: expected 3 arguments, got {}", args.len())),
    }
}

/// `len(x)` — element count. For strings: Unicode scalar count (not bytes).
/// For arrays: number of items.
fn builtin_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(s)] => Ok(Value::Int(s.chars().count() as i64)),
        [Value::Array(items)] => Ok(Value::Int(items.len() as i64)),
        [other] => Err(format!("len: expected string or array, got {}", other)),
        _ => Err(format!("len: expected 1 argument, got {}", args.len())),
    }
}

/// RES-034: write `value` into `items` at the path described by
/// `indices`. `indices[0]` indexes the outermost array; the last
/// index targets the leaf cell that gets replaced. Bounds errors
/// name the depth (1-indexed) where the out-of-range access occurred
/// so users can tell `m[2][0]` (outer) from `m[0][5]` (inner).
fn replace_at_path(
    items: &mut [Value],
    indices: &[i64],
    value: Value,
) -> RResult<()> {
    fn recurse(
        items: &mut [Value],
        indices: &[i64],
        value: Value,
        depth: usize,
    ) -> RResult<()> {
        let (i, rest) = match indices.split_first() {
            Some(pair) => pair,
            None => unreachable!("replace_at_path called with zero indices"),
        };
        if *i < 0 || (*i as usize) >= items.len() {
            return Err(format!(
                "Index {} out of bounds for array of length {} at dim {}",
                i,
                items.len(),
                depth
            ));
        }
        if rest.is_empty() {
            items[*i as usize] = value;
            return Ok(());
        }
        // Need to dive into the inner array. Move it out, recurse on
        // the inner Vec, then put the rebuilt array back. Cloning is
        // unnecessary because `items[i]` will be overwritten with
        // exactly the same Value::Array variant once the inner call
        // returns.
        let mut inner = std::mem::replace(&mut items[*i as usize], Value::Void);
        let Value::Array(inner_items) = &mut inner else {
            return Err(format!(
                "Cannot index into non-array at dim {}: {:?}",
                depth, inner
            ));
        };
        let result = recurse(inner_items, rest, value, depth + 1);
        items[*i as usize] = inner;
        result
    }
    recurse(items, indices, value, 1)
}

/// `abs(x)` — absolute value for `int` and `float`.
fn builtin_abs(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Int(i.abs())),
        [Value::Float(f)] => Ok(Value::Float(f.abs())),
        [other] => Err(format!("abs: expected int or float, got {}", other)),
        _ => Err(format!("abs: expected 1 argument, got {}", args.len())),
    }
}

/// `min(a, b)` — smaller of two numeric values. Coerces int↔float.
fn builtin_min(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(a), Value::Int(b)] => Ok(Value::Int((*a).min(*b))),
        [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a.min(*b))),
        [Value::Int(a), Value::Float(b)] => Ok(Value::Float((*a as f64).min(*b))),
        [Value::Float(a), Value::Int(b)] => Ok(Value::Float(a.min(*b as f64))),
        [a, b] => Err(format!("min: expected numeric args, got {:?} and {:?}", a, b)),
        _ => Err(format!("min: expected 2 arguments, got {}", args.len())),
    }
}

/// `max(a, b)` — larger of two numeric values. Coerces int↔float.
fn builtin_max(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(a), Value::Int(b)] => Ok(Value::Int((*a).max(*b))),
        [Value::Float(a), Value::Float(b)] => Ok(Value::Float(a.max(*b))),
        [Value::Int(a), Value::Float(b)] => Ok(Value::Float((*a as f64).max(*b))),
        [Value::Float(a), Value::Int(b)] => Ok(Value::Float(a.max(*b as f64))),
        [a, b] => Err(format!("max: expected numeric args, got {:?} and {:?}", a, b)),
        _ => Err(format!("max: expected 2 arguments, got {}", args.len())),
    }
}

/// RES-130: explicit widening from int to float.
fn builtin_to_float(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Float(*i as f64)),
        [Value::Float(f)] => Ok(Value::Float(*f)),
        [other] => Err(format!(
            "to_float: expected Int or Float argument, got {:?}",
            other
        )),
        _ => Err(format!(
            "to_float: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// RES-130: explicit narrowing from float to int. Truncates toward
/// zero (matches Rust's `as i64` cast semantics for finite values).
/// `NaN` and ±∞ are **runtime errors** rather than silent garbage —
/// propagating `i64::MIN` for NaN or clamping to `i64::MAX` for
/// +∞ would be the same kind of invisible bug this language exists
/// to eliminate.
fn builtin_to_int(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Int(i)] => Ok(Value::Int(*i)),
        [Value::Float(f)] => {
            if f.is_nan() {
                return Err("to_int: cannot convert NaN to int".to_string());
            }
            if f.is_infinite() {
                return Err(format!(
                    "to_int: cannot convert {} infinity to int",
                    if *f > 0.0 { "positive" } else { "negative" }
                ));
            }
            // `f as i64` saturates on out-of-range finite values in
            // Rust — that silent saturation is exactly the invisible
            // bug we're trying to avoid, so reject it too.
            if *f < (i64::MIN as f64) || *f > (i64::MAX as f64) {
                return Err(format!(
                    "to_int: value {} is out of i64 range",
                    f
                ));
            }
            Ok(Value::Int(*f as i64))
        }
        [other] => Err(format!(
            "to_int: expected Int or Float argument, got {:?}",
            other
        )),
        _ => Err(format!(
            "to_int: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// RES-143: `file_read(path)` — read a file as a UTF-8 string. Only
/// lives in the `resilient` CLI binary (which is std-only); the
/// `resilient-runtime` sibling crate has no builtins table and stays
/// no_std-clean. Errors surface as runtime diagnostics; the
/// interpreter wraps them with the call-site span (RES-116).
///
/// Security: the CLI has ambient authority over the filesystem, and
/// this builtin inherits it with no sandbox. Users running untrusted
/// Resilient programs should drop them in a chroot / container.
fn builtin_file_read(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path)] => match fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(contents) => Ok(Value::String(contents)),
                Err(_) => Err(format!("file_read: {} is not valid UTF-8", path)),
            },
            Err(e) => Err(format!("file_read: {}: {}", path, e)),
        },
        [other] => Err(format!(
            "file_read: expected String argument, got {}",
            other
        )),
        _ => Err(format!(
            "file_read: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// RES-151: `env(key: String) -> Result<String, String>` — read an
/// environment variable. `Ok(val)` when present, `Err("not set")`
/// when absent. Deliberately returns a Result so "the variable is
/// missing" is a first-class programmable outcome, not a runtime
/// halt.
///
/// Read-only by design: there is no matching `set_env` builtin.
/// Mutating a process's environment at runtime is a threading
/// footgun on hosts (Rust's `std::env::set_var` is `unsafe` on
/// recent editions for the same reason). std-only; the no_std
/// `resilient-runtime` sibling never gets this.
///
/// A non-UTF-8 environment value also surfaces as
/// `Err("invalid utf-8")` rather than a panic — consistent with
/// the ticket's spirit that absence isn't a runtime error.
fn builtin_env(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(key)] => {
            match std::env::var(key) {
                Ok(val) => Ok(Value::Result {
                    ok: true,
                    payload: Box::new(Value::String(val)),
                }),
                Err(std::env::VarError::NotPresent) => Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String("not set".to_string())),
                }),
                Err(std::env::VarError::NotUnicode(_)) => Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String(
                        "invalid utf-8".to_string(),
                    )),
                }),
            }
        }
        [other] => Err(format!(
            "env: expected String argument, got {}",
            other
        )),
        _ => Err(format!(
            "env: expected 1 argument (key), got {}",
            args.len()
        )),
    }
}

/// RES-143: `file_write(path, contents)` — write-truncate. Returns
/// `Void`. Errors surface as runtime diagnostics. Same security
/// posture as `file_read`.
fn builtin_file_write(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path), Value::String(contents)] => match fs::write(path, contents) {
            Ok(()) => Ok(Value::Void),
            Err(e) => Err(format!("file_write: {}: {}", path, e)),
        },
        [a, b] => Err(format!(
            "file_write: expected (String, String), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!(
            "file_write: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// --- RES-148: Map builtins ---

/// `map_new()` — produce an empty map.
fn builtin_map_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "map_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Map(std::collections::HashMap::new()))
}

/// `map_insert(m, k, v)` — insert / overwrite and return the updated
/// map. The argument map is cloned so the caller's binding is
/// unaffected, matching the immutable-value conventions elsewhere in
/// the interpreter (`push` on arrays returns a new array).
fn builtin_map_insert(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m), k, v] => {
            let key = MapKey::from_value(k)?;
            let mut out = m.clone();
            out.insert(key, v.clone());
            Ok(Value::Map(out))
        }
        [a, _, _] => Err(format!(
            "map_insert: first argument must be a Map, got {}",
            a
        )),
        _ => Err(format!(
            "map_insert: expected 3 arguments (map, key, value), got {}",
            args.len()
        )),
    }
}

/// `map_get(m, k) -> Result<V, Err>` — `Ok(v)` when present,
/// `Err("not found")` when absent.
fn builtin_map_get(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m), k] => {
            let key = MapKey::from_value(k)?;
            match m.get(&key) {
                Some(v) => Ok(Value::Result {
                    ok: true,
                    payload: Box::new(v.clone()),
                }),
                None => Ok(Value::Result {
                    ok: false,
                    payload: Box::new(Value::String("not found".to_string())),
                }),
            }
        }
        [a, _] => Err(format!(
            "map_get: first argument must be a Map, got {}",
            a
        )),
        _ => Err(format!(
            "map_get: expected 2 arguments (map, key), got {}",
            args.len()
        )),
    }
}

/// `map_remove(m, k)` — return the map with the key removed. Missing
/// keys are silently ignored (no-op), matching
/// `HashMap::remove`'s "Option<V>" shape interpreted in the
/// remove-for-side-effect direction.
fn builtin_map_remove(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m), k] => {
            let key = MapKey::from_value(k)?;
            let mut out = m.clone();
            out.remove(&key);
            Ok(Value::Map(out))
        }
        [a, _] => Err(format!(
            "map_remove: first argument must be a Map, got {}",
            a
        )),
        _ => Err(format!(
            "map_remove: expected 2 arguments (map, key), got {}",
            args.len()
        )),
    }
}

/// `map_keys(m) -> Array<K>` — keys in deterministic sort order so
/// golden tests and downstream logic don't observe HashMap's random
/// iteration.
fn builtin_map_keys(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => {
            let mut keys: Vec<&MapKey> = m.keys().collect();
            keys.sort_by(|a, b| match (a, b) {
                (MapKey::Int(x), MapKey::Int(y)) => x.cmp(y),
                (MapKey::Str(x), MapKey::Str(y)) => x.cmp(y),
                (MapKey::Bool(x), MapKey::Bool(y)) => x.cmp(y),
                (MapKey::Int(_), _) => std::cmp::Ordering::Less,
                (_, MapKey::Int(_)) => std::cmp::Ordering::Greater,
                (MapKey::Str(_), _) => std::cmp::Ordering::Less,
                (_, MapKey::Str(_)) => std::cmp::Ordering::Greater,
            });
            let out: Vec<Value> = keys.iter().map(|k| k.to_value()).collect();
            Ok(Value::Array(out))
        }
        [a] => Err(format!(
            "map_keys: expected a Map, got {}",
            a
        )),
        _ => Err(format!(
            "map_keys: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `map_len(m) -> Int` — number of entries.
fn builtin_map_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Map(m)] => Ok(Value::Int(m.len() as i64)),
        [a] => Err(format!(
            "map_len: expected a Map, got {}",
            a
        )),
        _ => Err(format!(
            "map_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// --- RES-152: Bytes builtins ---
//
// Three immutable accessors: length, range slice, and per-byte
// indexed read. Mirrors the Array shape (`len`, `slice`) with a
// distinct naming prefix so Bytes / Array stay in separate
// name-spaces. `byte_at` returns `Int` (i64) — the language has
// no `u8` type and narrowing belongs to a future fixed-width
// ticket per RES-152's Notes.

/// `bytes_len(b) -> Int` — number of bytes.
fn builtin_bytes_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b)] => Ok(Value::Int(b.len() as i64)),
        [other] => Err(format!(
            "bytes_len: expected Bytes, got {}",
            other
        )),
        _ => Err(format!(
            "bytes_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `bytes_slice(b, start, end) -> Bytes` — half-open `[start, end)`
/// slice. Errors on reversed bounds or out-of-range indices with a
/// runtime diagnostic; callers get a span via the interpreter's
/// error-wrapping layer (RES-116).
fn builtin_bytes_slice(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(start), Value::Int(end)] => {
            let len = b.len() as i64;
            if *start < 0 || *end < 0 {
                return Err(format!(
                    "bytes_slice: negative index — start={}, end={}",
                    start, end
                ));
            }
            if start > end {
                return Err(format!(
                    "bytes_slice: start must be <= end — got start={}, end={}",
                    start, end
                ));
            }
            if *end > len {
                return Err(format!(
                    "bytes_slice: end {} out of range for Bytes of length {}",
                    end, len
                ));
            }
            let out: Vec<u8> = b[(*start as usize)..(*end as usize)].to_vec();
            Ok(Value::Bytes(out))
        }
        [a, b, c] => Err(format!(
            "bytes_slice: expected (Bytes, Int, Int), got ({:?}, {:?}, {:?})",
            a, b, c
        )),
        _ => Err(format!(
            "bytes_slice: expected 3 arguments (bytes, start, end), got {}",
            args.len()
        )),
    }
}

/// `byte_at(b, i) -> Int` — returns the i-th byte as an Int in
/// `0..=255`. Out-of-range `i` is a runtime error.
fn builtin_byte_at(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Bytes(b), Value::Int(i)] => {
            if *i < 0 || (*i as usize) >= b.len() {
                return Err(format!(
                    "byte_at: index {} out of range for Bytes of length {}",
                    i,
                    b.len()
                ));
            }
            Ok(Value::Int(b[*i as usize] as i64))
        }
        [a, b] => Err(format!(
            "byte_at: expected (Bytes, Int), got ({:?}, {:?})",
            a, b
        )),
        _ => Err(format!(
            "byte_at: expected 2 arguments (bytes, index), got {}",
            args.len()
        )),
    }
}

// --- RES-149: Set builtins ---
//
// Value restrictions mirror Map: Int / String / Bool only. Immutable
// at the Value layer — `set_insert` / `set_remove` return new sets,
// matching `push` / `map_insert` conventions.

/// `set_new()` — produce an empty set.
fn builtin_set_new(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "set_new: expected 0 arguments, got {}",
            args.len()
        ));
    }
    Ok(Value::Set(std::collections::HashSet::new()))
}

/// `set_insert(s, x) -> Set` — return the set with `x` added (no-op
/// if already present). Input set is cloned so the caller's binding
/// is unaffected.
fn builtin_set_insert(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s), x] => {
            let k = MapKey::from_value(x).map_err(|e| {
                e.replace("Map key", "Set element")
            })?;
            let mut out = s.clone();
            out.insert(k);
            Ok(Value::Set(out))
        }
        [a, _] => Err(format!(
            "set_insert: first argument must be a Set, got {}",
            a
        )),
        _ => Err(format!(
            "set_insert: expected 2 arguments (set, element), got {}",
            args.len()
        )),
    }
}

/// `set_remove(s, x) -> Set` — return the set without `x`. Absent
/// elements are silently ignored.
fn builtin_set_remove(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s), x] => {
            let k = MapKey::from_value(x).map_err(|e| {
                e.replace("Map key", "Set element")
            })?;
            let mut out = s.clone();
            out.remove(&k);
            Ok(Value::Set(out))
        }
        [a, _] => Err(format!(
            "set_remove: first argument must be a Set, got {}",
            a
        )),
        _ => Err(format!(
            "set_remove: expected 2 arguments (set, element), got {}",
            args.len()
        )),
    }
}

/// `set_has(s, x) -> Bool` — membership test.
fn builtin_set_has(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s), x] => {
            let k = MapKey::from_value(x).map_err(|e| {
                e.replace("Map key", "Set element")
            })?;
            Ok(Value::Bool(s.contains(&k)))
        }
        [a, _] => Err(format!(
            "set_has: first argument must be a Set, got {}",
            a
        )),
        _ => Err(format!(
            "set_has: expected 2 arguments (set, element), got {}",
            args.len()
        )),
    }
}

/// `set_len(s) -> Int` — cardinality.
fn builtin_set_len(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s)] => Ok(Value::Int(s.len() as i64)),
        [a] => Err(format!(
            "set_len: expected a Set, got {}",
            a
        )),
        _ => Err(format!(
            "set_len: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `set_items(s) -> Array<T>` — lift elements out into an array so
/// array-consumers (`for ... in`, comprehensions via RES-156) work
/// on sets without extra syntax. Order is deterministic (sorted),
/// same rationale as `map_keys`.
fn builtin_set_items(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Set(s)] => {
            let mut items: Vec<&MapKey> = s.iter().collect();
            items.sort_by(|a, b| match (a, b) {
                (MapKey::Int(x), MapKey::Int(y)) => x.cmp(y),
                (MapKey::Str(x), MapKey::Str(y)) => x.cmp(y),
                (MapKey::Bool(x), MapKey::Bool(y)) => x.cmp(y),
                (MapKey::Int(_), _) => std::cmp::Ordering::Less,
                (_, MapKey::Int(_)) => std::cmp::Ordering::Greater,
                (MapKey::Str(_), _) => std::cmp::Ordering::Less,
                (_, MapKey::Str(_)) => std::cmp::Ordering::Greater,
            });
            let out: Vec<Value> = items.iter().map(|k| k.to_value()).collect();
            Ok(Value::Array(out))
        }
        [a] => Err(format!(
            "set_items: expected a Set, got {}",
            a
        )),
        _ => Err(format!(
            "set_items: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// --- RES-138: live-block retry-counter thread-local ---
//
// `live_retries()` inside a `live { ... }` body needs to read
// the current retry counter. A thread-local stack holds one
// `usize` per active live block (top = innermost), let through
// the builtin. The RAII `LiveRetryGuard` guarantees the stack
// pops on every exit path from `eval_live_block` — success, max-
// retry-exhausted, or panic — so the stack can't leak across
// blocks.

thread_local! {
    static LIVE_RETRY_STACK: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

struct LiveRetryGuard;

impl LiveRetryGuard {
    fn enter() -> Self {
        LIVE_RETRY_STACK.with(|s| s.borrow_mut().push(0));
        LiveRetryGuard
    }

    /// Update the innermost stack entry (the one `live_retries()`
    /// will read) to `count`. Called from the retry branch of
    /// `eval_live_block` after incrementing the local counter.
    fn set(count: usize) {
        LIVE_RETRY_STACK.with(|s| {
            if let Some(top) = s.borrow_mut().last_mut() {
                *top = count;
            }
        });
    }
}

impl Drop for LiveRetryGuard {
    fn drop(&mut self) {
        LIVE_RETRY_STACK.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

// --- RES-141: process-wide live-block telemetry counters ---
//
// Two `AtomicU32`s that accumulate across the whole `resilient`
// run. `LIVE_TOTAL_RETRIES` bumps every time an inner body fails
// and the block schedules a retry; `LIVE_TOTAL_EXHAUSTIONS` bumps
// every time a block gives up after `MAX_RETRIES`. Reads use
// `Relaxed` ordering — for diagnostic-quality counters we only
// need eventual visibility, not a happens-before guarantee.
//
// `u32` (not `u64`) so the same counter shape works on ARM
// Cortex-M where only 32-bit atomics are native. 2^32 retries is
// plenty of headroom for diagnostic runs.

static LIVE_TOTAL_RETRIES: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);
static LIVE_TOTAL_EXHAUSTIONS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// RES-141: `live_total_retries() -> Int` — cumulative count of
/// live-block retries since the process started.
fn builtin_live_total_retries(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "live_total_retries: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let n = LIVE_TOTAL_RETRIES.load(std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Int(n as i64))
}

/// RES-141: `live_total_exhaustions() -> Int` — cumulative count
/// of live blocks that exhausted their retry budget.
fn builtin_live_total_exhaustions(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "live_total_exhaustions: expected 0 arguments, got {}",
            args.len()
        ));
    }
    let n = LIVE_TOTAL_EXHAUSTIONS.load(std::sync::atomic::Ordering::Relaxed);
    Ok(Value::Int(n as i64))
}

/// RES-138: `live_retries() -> Int`. Inside a `live { ... }`
/// block returns the current retry count (0 on the first
/// attempt, 1..∞ thereafter). Nested `live` blocks read the
/// innermost block's counter. Called outside any live block,
/// returns a clean runtime error.
fn builtin_live_retries(args: &[Value]) -> RResult<Value> {
    if !args.is_empty() {
        return Err(format!(
            "live_retries: expected 0 arguments, got {}",
            args.len()
        ));
    }
    LIVE_RETRY_STACK.with(|s| match s.borrow().last() {
        Some(&n) => Ok(Value::Int(n as i64)),
        None => Err("live_retries() called outside a live block".to_string()),
    })
}

// Interpreter for executing Resilient programs
struct Interpreter {
    env: Environment,
    /// RES-013: static-let bindings. Shared across every sub-interpreter
    /// created for function calls so the values survive across invocations.
    /// Keyed by the static's identifier (caveat: two functions using the
    /// same static name currently share — good enough for MVP).
    statics: Rc<RefCell<HashMap<String, Value>>>,
    /// RES-068: function names whose `requires` clauses were 100%
    /// statically discharged across every observed call site. The
    /// runtime check for these is provably redundant — when binding a
    /// Function value, we strip the requires so apply_function skips
    /// the runtime check entirely. Empty by default; populated by
    /// `with_proven_fns` after typecheck.
    proven_fns: Rc<HashSet<String>>,
}

impl Interpreter {
    fn new() -> Self {
        let mut env = Environment::new();
        register_builtins(&mut env);
        Interpreter {
            env,
            statics: Rc::new(RefCell::new(HashMap::new())),
            proven_fns: Rc::new(HashSet::new()),
        }
    }

    /// RES-068: pass the set of fully-proven function names to the
    /// interpreter. Their `requires` clauses won't fire at runtime.
    fn with_proven_fns(mut self, proven: HashSet<String>) -> Self {
        self.proven_fns = Rc::new(proven);
        self
    }
    
    fn eval(&mut self, node: &Node) -> RResult<Value> {
        match node {
            Node::Program(statements) => self.eval_program(statements),
            // RES-073: `use` should have been resolved by expand_uses
            // before the program reached here. Treat any leftover as
            // a no-op so unit tests that bypass the driver don't trip.
            Node::Use { .. } => Ok(Value::Void),
            Node::Function { name, parameters, body, requires, ensures, .. } => {
                // RES-068: if every observed call site for this fn was
                // statically proven, the runtime requires check is
                // provably redundant. Strip it.
                let runtime_requires = if self.proven_fns.contains(name) {
                    Vec::new()
                } else {
                    requires.clone()
                };
                let func = Value::Function {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                    requires: runtime_requires,
                    ensures: ensures.clone(),
                    name: name.clone(),
                };
                self.env.set(name.clone(), func);
                Ok(Value::Void)
            },
            Node::LiveBlock { body, invariants, backoff, timeout, .. } => {
                // RES-142: unpack the `within <duration>` clause
                // (if any) into a flat `u64 ns` so the runtime loop
                // doesn't re-match the boxed node every retry.
                let timeout_ns = timeout.as_ref().and_then(|n| match n.as_ref() {
                    Node::DurationLiteral { nanos, .. } => Some(*nanos),
                    _ => None,
                });
                self.eval_live_block(body, invariants, backoff.as_ref(), timeout_ns)
            }
            // RES-142: duration literals are only legal inside a
            // `live ... within <duration> { ... }` clause — the
            // parser never emits them in general expression
            // position. If one reaches eval, it's an internal bug
            // (or a test building an AST by hand). Fail loudly
            // rather than silently evaluating to an Int.
            Node::DurationLiteral { .. } => Err(
                "duration literals are only valid inside `live within ...` clauses (RES-142)"
                    .to_string(),
            ),
            Node::Assert { condition, message, .. } => self.eval_assert(condition, message),
            Node::Block { stmts: statements, .. } => self.eval_block_statement(statements),
            Node::LetStatement { name, value, .. } => {
                let val = self.eval(value)?;
                // RES-041: if the RHS short-circuited (e.g. via `?`),
                // propagate the Return instead of binding it.
                if matches!(val, Value::Return(_)) {
                    return Ok(val);
                }
                self.env.set(name.clone(), val);
                Ok(Value::Void)
            },
            Node::LetDestructureStruct {
                struct_name,
                fields,
                value,
                ..
            } => {
                let val = self.eval(value)?;
                if matches!(val, Value::Return(_)) {
                    return Ok(val);
                }
                let (obs_name, obs_fields) = match &val {
                    Value::Struct { name, fields } => (name.clone(), fields.clone()),
                    other => {
                        return Err(format!(
                            "Cannot destructure non-struct value as {}: got {}",
                            struct_name, other
                        ));
                    }
                };
                if obs_name != *struct_name {
                    return Err(format!(
                        "Destructure expected struct {}, got {}",
                        struct_name, obs_name
                    ));
                }
                // Bind each requested field into the environment.
                for (field_name, local_name) in fields {
                    let Some((_, field_val)) =
                        obs_fields.iter().find(|(n, _)| n == field_name)
                    else {
                        return Err(format!(
                            "Struct {} has no field `{}`",
                            struct_name, field_name
                        ));
                    };
                    self.env.set(local_name.clone(), field_val.clone());
                }
                Ok(Value::Void)
            }
            Node::StaticLet { name, value, .. } => {
                // Initialize only once. Subsequent executions of the
                // same declaration are no-ops (the value persists in
                // self.statics across function calls).
                if !self.statics.borrow().contains_key(name) {
                    let val = self.eval(value)?;
                    self.statics.borrow_mut().insert(name.clone(), val);
                }
                Ok(Value::Void)
            },
            Node::Assignment { name, value, .. } => {
                let val = self.eval(value)?;
                if matches!(val, Value::Return(_)) {
                    return Ok(val);
                }
                if self.env.reassign(name, val.clone()) {
                    Ok(Value::Void)
                } else if self.statics.borrow().contains_key(name) {
                    self.statics.borrow_mut().insert(name.clone(), val);
                    Ok(Value::Void)
                } else {
                    Err(format!("Cannot assign to undeclared variable '{}'", name))
                }
            },
            Node::ReturnStatement { value, .. } => {
                let val = match value {
                    Some(expr) => self.eval(expr)?,
                    None => Value::Void,
                };
                Ok(Value::Return(Box::new(val)))
            },
            Node::ForInStatement { name, iterable, body, .. } => {
                let iter_val = self.eval(iterable)?;
                let items = match iter_val {
                    Value::Array(v) => v,
                    other => return Err(format!(
                        "`for` iterable must be an array, got {}",
                        other
                    )),
                };
                for item in items {
                    self.env.set(name.clone(), item);
                    let result = self.eval(body)?;
                    if let Value::Return(_) = result {
                        return Ok(result);
                    }
                }
                Ok(Value::Void)
            },
            Node::WhileStatement { condition, body, .. } => {
                // Cap iterations as a safety net so a buggy loop can't
                // freeze the interpreter. 1M is big enough for
                // realistic work and small enough to catch runaways.
                const MAX_ITERS: usize = 1_000_000;
                let mut iters = 0usize;
                loop {
                    iters += 1;
                    if iters > MAX_ITERS {
                        return Err(format!(
                            "while loop exceeded {MAX_ITERS} iterations (runaway?)"
                        ));
                    }
                    let cond_val = self.eval(condition)?;
                    if !self.is_truthy(&cond_val) {
                        break;
                    }
                    let result = self.eval(body)?;
                    if let Value::Return(_) = result {
                        return Ok(result);
                    }
                }
                Ok(Value::Void)
            },
            Node::IfStatement { condition, consequence, alternative, .. } => {
                let condition_value = self.eval(condition)?;
                if self.is_truthy(&condition_value) {
                    self.eval(consequence)
                } else if let Some(alt) = alternative {
                    self.eval(alt)
                } else {
                    Ok(Value::Void)
                }
            },
            Node::ExpressionStatement { expr, .. } => self.eval(expr),
            Node::Identifier { name, .. } => {
                if let Some(value) = self.env.get(name) {
                    Ok(value)
                } else if let Some(value) = self.statics.borrow().get(name).cloned() {
                    Ok(value)
                } else {
                    Err(format!("Identifier not found: {}", name))
                }
            },
            Node::IntegerLiteral { value, .. } => Ok(Value::Int(*value)),
            Node::FloatLiteral { value, .. } => Ok(Value::Float(*value)),
            Node::StringLiteral { value, .. } => Ok(Value::String(value.clone())),
            Node::BytesLiteral { value, .. } => Ok(Value::Bytes(value.clone())),
            Node::BooleanLiteral { value, .. } => Ok(Value::Bool(*value)),
            Node::PrefixExpression { operator, right, .. } => {
                let right_val = self.eval(right)?;
                self.eval_prefix_expression(operator, right_val)
            },
            Node::InfixExpression { left, operator, right, .. } => {
                let left_val = self.eval(left)?;
                let right_val = self.eval(right)?;
                self.eval_infix_expression(operator, left_val, right_val)
            },
            Node::CallExpression { function, arguments, .. } => {
                // RES-158: method call desugar.
                //
                // If the callee is `TARGET.NAME`, evaluate `TARGET`
                // first; if the result is a struct value, look up
                // `<Struct>$<NAME>` in the environment and treat the
                // call as `<Struct>$<NAME>(TARGET, ...args)`. This is
                // the pure sugar layer — once dispatched, the method
                // is an ordinary function invocation.
                if let Node::FieldAccess { target, field, .. } = function.as_ref() {
                    let target_val = self.eval(target)?;
                    if let Value::Struct { name: sname, .. } = &target_val {
                        let mangled = format!("{}${}", sname, field);
                        if let Some(method_val) = self.env.get(&mangled) {
                            // Prepend the target as the implicit `self`.
                            let mut args = vec![target_val];
                            args.extend(self.eval_expressions(arguments)?);
                            return self.apply_function(method_val, args);
                        }
                        // No matching method on this struct — fall
                        // through to the regular `FieldAccess` eval
                        // (which will itself raise a clean error).
                    }
                }
                let func = self.eval(function)?;
                let args = self.eval_expressions(arguments)?;
                self.apply_function(func, args)
            },
            Node::ArrayLiteral { items, .. } => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item)?);
                }
                Ok(Value::Array(out))
            },
            // RES-148: `{k -> v, ...}` — evaluate each key / value and
            // coerce keys to `MapKey` (errors if a key isn't one of
            // the hashable primitives). Later insertions on the same
            // key overwrite earlier ones, matching HashMap's semantics.
            Node::MapLiteral { entries, .. } => {
                let mut m: std::collections::HashMap<MapKey, Value> =
                    std::collections::HashMap::with_capacity(entries.len());
                for (k_node, v_node) in entries {
                    let k_val = self.eval(k_node)?;
                    let v_val = self.eval(v_node)?;
                    let k = MapKey::from_value(&k_val)?;
                    m.insert(k, v_val);
                }
                Ok(Value::Map(m))
            }
            Node::SetLiteral { items, .. } => {
                // RES-149: build a HashSet<MapKey> by evaluating
                // each item and coercing via the same MapKey
                // restriction as map keys — Int / String / Bool.
                // Duplicates are collapsed by `HashSet::insert`.
                let mut set: std::collections::HashSet<MapKey> =
                    std::collections::HashSet::with_capacity(items.len());
                for item in items {
                    let v = self.eval(item)?;
                    let k = MapKey::from_value(&v).map_err(|e| {
                        // Surface "Set element" in the diagnostic
                        // rather than "Map key" — same restriction,
                        // different context.
                        e.replace("Map key", "Set element")
                    })?;
                    set.insert(k);
                }
                Ok(Value::Set(set))
            }
            Node::FunctionLiteral { parameters, body, requires, ensures, .. } => {
                Ok(Value::Function {
                    parameters: parameters.clone(),
                    body: body.clone(),
                    env: self.env.clone(),
                    requires: requires.clone(),
                    ensures: ensures.clone(),
                    name: "<anon>".to_string(),
                })
            },
            Node::TryExpression { expr: inner, .. } => {
                let v = self.eval(inner)?;
                match v {
                    Value::Result { ok: true, payload } => Ok(*payload),
                    Value::Result { ok: false, payload } => {
                        // Propagate: return Err(payload) from the
                        // enclosing function via the Value::Return
                        // short-circuit path already used by `return`.
                        Ok(Value::Return(Box::new(Value::Result {
                            ok: false,
                            payload,
                        })))
                    }
                    other => Err(format!(
                        "? operator expects a Result, got {}",
                        other
                    )),
                }
            },
            Node::Match { scrutinee, arms, .. } => {
                let sval = self.eval(scrutinee)?;
                for (pattern, guard, body) in arms {
                    if let Some(binding) = self.match_pattern(pattern, &sval)? {
                        // RES-159: evaluate the arm with any pattern
                        // binding in scope first, then check the
                        // guard (so the guard can reference the
                        // binding). A `false` guard falls through to
                        // the next arm; a true / absent guard fires
                        // the body in the same scope. Either way,
                        // the scoped env is restored on exit so the
                        // binding doesn't leak.
                        let saved = self.env.clone();
                        let mut had_scope = false;
                        if let Some((name, value)) = binding {
                            self.env = Environment::new_enclosed(saved.clone());
                            self.env.set(name, value);
                            had_scope = true;
                        }
                        let guard_pass = match guard {
                            Some(g) => {
                                let gv = self.eval(g);
                                match gv {
                                    Ok(v) => self.is_truthy(&v),
                                    Err(e) => {
                                        if had_scope { self.env = saved.clone(); }
                                        return Err(e);
                                    }
                                }
                            }
                            None => true,
                        };
                        if !guard_pass {
                            if had_scope { self.env = saved; }
                            continue; // try next arm
                        }
                        let result = self.eval(body);
                        if had_scope { self.env = saved; }
                        return result;
                    }
                }
                // No arm matched → void.
                Ok(Value::Void)
            },
            Node::StructDecl { .. } => {
                // Declarations are pure compile-time metadata today.
                // The typechecker (G7) will register them in a struct
                // table; for now they're a runtime no-op, and Value
                // construction trusts the literal.
                Ok(Value::Void)
            },
            // RES-128: `type NAME = TARGET;` is purely a
            // typechecker / documentation concern. Runtime never
            // sees aliases — by the time `eval` runs, every use
            // site has already been resolved by the typechecker.
            Node::TypeAlias { .. } => Ok(Value::Void),
            // RES-158: `impl <Struct> { ... }` evaluates each method
            // as if it were a top-level `fn` decl. Methods are already
            // mangled to `<Struct>$<method>` by the parser.
            Node::ImplBlock { methods, struct_name, .. } => {
                for method in methods {
                    // Detect duplicate-method-across-blocks: if the
                    // mangled name already resolves to a user Function
                    // in the current env, error — the ticket calls this
                    // out as a duplicate-def diagnostic.
                    if let Node::Function { name: mangled, .. } = method
                        && matches!(self.env.get(mangled), Some(Value::Function { .. }))
                    {
                        return Err(format!(
                            "duplicate method: `{}::{}` defined more than once across impl blocks",
                            struct_name,
                            mangled.strip_prefix(&format!("{}$", struct_name)).unwrap_or(mangled),
                        ));
                    }
                    self.eval(method)?;
                }
                Ok(Value::Void)
            },
            Node::StructLiteral { name, fields, .. } => {
                let mut out = Vec::with_capacity(fields.len());
                for (fname, fexpr) in fields {
                    out.push((fname.clone(), self.eval(fexpr)?));
                }
                Ok(Value::Struct {
                    name: name.clone(),
                    fields: out,
                })
            },
            Node::FieldAccess { target, field, .. } => {
                let tval = self.eval(target)?;
                match tval {
                    Value::Struct { name, fields } => {
                        fields
                            .into_iter()
                            .find(|(n, _)| n == field)
                            .map(|(_, v)| v)
                            .ok_or_else(|| {
                                format!("Struct {} has no field '{}'", name, field)
                            })
                    }
                    other => Err(format!(
                        "Cannot access field '{}' on non-struct {:?}",
                        field, other
                    )),
                }
            },
            Node::FieldAssignment { target, field, value, .. } => {
                // Only support `IDENT.field = v` and `IDENT.f1.f2 = v`
                // for MVP. The target tree is a chain of Identifier and
                // FieldAccess nodes; we walk it to find the root binding,
                // then mutate a cloned copy and reassign.
                let new_val = self.eval(value)?;
                let (root_name, path) = flatten_field_target(target, field);
                let Some(root_name) = root_name else {
                    return Err(
                        "Field assignment target must start with an identifier"
                            .to_string(),
                    );
                };
                let current = self
                    .env
                    .get(&root_name)
                    .ok_or_else(|| format!("Identifier not found: {}", root_name))?;
                let updated = set_nested_field(current, &path, new_val)?;
                let _ = self.env.reassign(&root_name, updated);
                Ok(Value::Void)
            },
            Node::IndexExpression { target, index, .. } => {
                let target_val = self.eval(target)?;
                let index_val = self.eval(index)?;
                match (target_val, index_val) {
                    (Value::Array(items), Value::Int(i)) => {
                        if i < 0 || (i as usize) >= items.len() {
                            Err(format!(
                                "Index {} out of bounds for array of length {}",
                                i,
                                items.len()
                            ))
                        } else {
                            Ok(items[i as usize].clone())
                        }
                    }
                    (Value::Array(_), other) => Err(format!(
                        "Array index must be int, got {}",
                        other
                    )),
                    (other, _) => Err(format!("Cannot index {:?}", other)),
                }
            },
            Node::IndexAssignment { target, index, value, .. } => {
                // RES-034: walk the LHS chain to support a[i][j]...[k] = v.
                // The parser builds nested IndexExpression nodes; descend
                // through them collecting each index (root-to-leaf order)
                // and the root identifier name.
                let mut indices_rev: Vec<&Node> = vec![index];
                let mut cursor: &Node = target;
                let root_name = loop {
                    match cursor {
                        Node::Identifier { name, .. } => break name.clone(),
                        Node::IndexExpression { target: inner_t, index: inner_i, .. } => {
                            indices_rev.push(inner_i);
                            cursor = inner_t;
                        }
                        _ => {
                            return Err(
                                "Index assignment target must be an identifier".to_string()
                            );
                        }
                    }
                };
                // We collected leaf-first; reverse to root-first so
                // path[0] indexes the outermost array.
                indices_rev.reverse();
                let path_exprs = indices_rev;

                // Evaluate the RHS first so any side effects there
                // happen before we start mutating the array.
                let new_val = self.eval(value)?;
                // Then evaluate every index expression in source order.
                let mut path_indices: Vec<i64> = Vec::with_capacity(path_exprs.len());
                for idx_expr in &path_exprs {
                    let idx_val = self.eval(idx_expr)?;
                    let Value::Int(i) = idx_val else {
                        return Err(format!("Array index must be int, got {}", idx_val));
                    };
                    path_indices.push(i);
                }

                // Read–modify–write. `env.get` returns a clone, so the
                // mutation is local until we `reassign` the new root
                // value — that preserves value semantics for sibling
                // bindings.
                let root = self
                    .env
                    .get(&root_name)
                    .ok_or_else(|| format!("Identifier not found: {}", root_name))?;
                let Value::Array(mut items) = root else {
                    return Err(format!(
                        "Cannot index-assign into non-array '{}'",
                        root_name
                    ));
                };
                replace_at_path(&mut items, &path_indices, new_val)?;
                let _ = self.env.reassign(&root_name, Value::Array(items));
                Ok(Value::Void)
            },
        }
    }
    
    fn eval_program(&mut self, statements: &[span::Spanned<Node>]) -> RResult<Value> {
        // RES-018 + RES-050: hoist top-level fn bindings so call sites
        // can forward-reference. ONE pass suffices now — captured envs
        // are Rc<RefCell> so the post-hoist mutation of the env is
        // visible to every previously-captured handle.
        // RES-077: statements are now Spanned<Node>; deref via .node.
        // RES-158: impl blocks hoist too — their contained methods
        // are plain Function decls under the hood, so `main()` can
        // freely call `p.method()` even if the impl block textually
        // follows `main`.
        for statement in statements {
            match &statement.node {
                Node::Function { .. } | Node::ImplBlock { .. } => {
                    self.eval(&statement.node)
                        .map_err(|e| decorate_runtime_error(e, &statement.span))?;
                }
                _ => {}
            }
        }

        let mut result = Value::Void;
        for statement in statements {
            if matches!(
                statement.node,
                Node::Function { .. } | Node::ImplBlock { .. }
            ) {
                continue;
            }
            // RES-116: decorate runtime errors with the statement's
            // source span so `execute_file` can reformat them as
            // `filename:line:col: Runtime error: <msg>` — matching the
            // VM's RES-091/092 output shape.
            result = self.eval(&statement.node)
                .map_err(|e| decorate_runtime_error(e, &statement.span))?;
            if let Value::Return(value) = result {
                return Ok(*value);
            }
        }

        Ok(result)
    }
    
    fn eval_block_statement(&mut self, statements: &[Node]) -> RResult<Value> {
        let mut result = Value::Void;
        
        for statement in statements {
            result = self.eval(statement)?;
            
            if let Value::Return(_) = result {
                return Ok(result);
            }
        }
        
        Ok(result)
    }
    
    fn eval_live_block(
        &mut self,
        body: &Node,
        invariants: &[Node],
        backoff: Option<&BackoffConfig>,
        timeout_ns: Option<u64>,
    ) -> RResult<Value> {
        const MAX_RETRIES: usize = 3;
        let mut retry_count = 0;

        // Create a snapshot of the environment
        // RES-050: now that env clone is shallow (Rc bump), we must
        // explicitly deep-clone here to preserve the live-block's
        // restore-on-retry semantics.
        let env_snapshot = self.env.deep_clone();

        // Log the start of live block execution
        eprintln!("\x1B[36m[LIVE BLOCK] Starting execution of live block\x1B[0m");

        // RES-138: push a fresh retry counter onto the thread-local
        // stack so `live_retries()` inside `body` / invariants can
        // read it. The RAII `_guard` drops on ALL exit paths
        // (success, max-retry-exhausted, panic), keeping the stack
        // from leaking across `live` blocks — including nested
        // ones, where the builtin reads the innermost block's top.
        let _guard = LiveRetryGuard::enter();

        // RES-142: wall-clock start for the `within <duration>`
        // budget. Sampled once at block entry so retries and backoff
        // sleeps both count against the same budget. `None` means
        // "no timeout" and the clock is never queried.
        let live_start = timeout_ns.map(|_| std::time::Instant::now());

        // Try to evaluate the body with multiple retries
        loop {
            // RES-036: treat an invariant failure as the same class of
            // recoverable error the retry loop already handles. The
            // body eval either succeeds or returns Err; then we check
            // each invariant and convert a false result into an Err.
            let outcome = self.eval(body).and_then(|value| {
                for clause in invariants {
                    let v = self.eval(clause)?;
                    if !self.is_truthy(&v) {
                        return Err(format!(
                            "Invariant violation in live block: {} failed",
                            format_contract_expr(clause)
                        ));
                    }
                }
                Ok(value)
            });

            match outcome {
                Ok(value) => {
                    eprintln!("\x1B[32m[LIVE BLOCK] Successfully executed live block\x1B[0m");
                    return Ok(value);
                }
                Err(error) => {
                    retry_count += 1;
                    // RES-138: keep the thread-local counter in
                    // sync so `live_retries()` inside the body on
                    // the NEXT attempt sees the current retry
                    // number (0 on first attempt, then 1, 2, …).
                    LiveRetryGuard::set(retry_count);
                    // RES-141: bump the process-wide retry counter
                    // only when we actually retry — an exhausting
                    // failure below doesn't retry, it gives up
                    // (that's the `exhaustions` counter's job).
                    // Relaxed is fine; counters are diagnostic-
                    // quality, not a synchronization primitive.
                    if retry_count < MAX_RETRIES {
                        LIVE_TOTAL_RETRIES
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }

                    eprintln!(
                        "\x1B[33m[LIVE BLOCK] Error detected (attempt {}/{}): {}\x1B[0m",
                        retry_count, MAX_RETRIES, error
                    );

                    // RES-142: budget check. If the wall-clock
                    // elapsed since block entry exceeds the
                    // `within <duration>` cap, escalate the same
                    // way exhaustion does (RES-140 footer + the
                    // `LIVE_TOTAL_EXHAUSTIONS` bump per RES-141) —
                    // "timed out" is just another flavour of
                    // giving up. Checked BEFORE the retry-cap and
                    // backoff sleep so an over-budget run bails
                    // immediately without an extra sleep.
                    let timed_out = match (live_start, timeout_ns) {
                        (Some(t0), Some(budget)) => {
                            let elapsed = t0.elapsed().as_nanos();
                            elapsed >= u128::from(budget)
                        }
                        _ => false,
                    };

                    if retry_count >= MAX_RETRIES || timed_out {
                        let reason = if timed_out {
                            "timed out"
                        } else {
                            "Maximum retry attempts reached"
                        };
                        eprintln!(
                            "\x1B[31m[LIVE BLOCK] {}, propagating error\x1B[0m",
                            reason
                        );
                        // RES-141: bump the exhaustion counter
                        // before returning — tracks how many
                        // times any live block gave up across the
                        // whole run. Timeout counts as exhaustion.
                        LIVE_TOTAL_EXHAUSTIONS
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // RES-140: footer note recording the
                        // nesting depth at which exhaustion fired.
                        // `LIVE_RETRY_STACK.len()` at this point
                        // is the current level (self included). As
                        // an inner block's error escalates up the
                        // call chain, each outer level wraps it
                        // with its OWN "after N attempts" prefix
                        // plus its own depth — together the nested
                        // wrappers serialize the retry-depth
                        // history at every nesting level.
                        let depth = LIVE_RETRY_STACK.with(|s| s.borrow().len());
                        // RES-142: timeout uses a distinct prefix
                        // so diagnostics can tell "gave up by
                        // retry cap" apart from "gave up by wall-
                        // clock budget".
                        if timed_out {
                            return Err(format!(
                                "Live block timed out after {} attempt(s) (retry depth: {}): {}",
                                retry_count, depth, error
                            ));
                        }
                        return Err(format!(
                            "Live block failed after {} attempts (retry depth: {}): {}",
                            MAX_RETRIES, depth, error
                        ));
                    }

                    eprintln!(
                        "\x1B[36m[LIVE BLOCK] Restoring environment to last known good state\x1B[0m"
                    );
                    eprintln!(
                        "\x1B[36m[LIVE BLOCK] Retrying execution (attempt {}/{})\x1B[0m",
                        retry_count + 1,
                        MAX_RETRIES
                    );

                    // RES-139: exponential backoff between retries.
                    // `retries` here is `retry_count - 1` so the
                    // first retry (after the first failure) sleeps
                    // `base_ms`, the second `base_ms * factor`,
                    // etc., capped at `max_ms`. `None` preserves
                    // the zero-sleep behaviour for plain `live { }`.
                    if let Some(cfg) = backoff {
                        let ms = cfg.delay_ms((retry_count - 1) as u32);
                        if ms > 0 {
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                        }
                    }

                    // Restore the environment from the snapshot
                    // Each retry gets a FRESH deep copy of the snapshot
                    // — otherwise the first retry's mutations would
                    // pollute the second.
                    self.env = env_snapshot.deep_clone();
                }
            }
        }
    }
    
    fn eval_assert(&mut self, condition: &Node, message: &Option<Box<Node>>) -> RResult<Value> {
        let condition_value = self.eval(condition)?;

        if !self.is_truthy(&condition_value) {
            let error_message = if let Some(msg) = message {
                match self.eval(msg)? {
                    Value::String(s) => s,
                    other => format!("Assertion failed with message: {}", other),
                }
            } else {
                "Assertion failed".to_string()
            };

            // RES-028: if the condition is a comparison, show both
            // operand values so "fuel >= 0" doesn't just say "false" —
            // it says "fuel = -5, 0 >= 0 — got: -5 >= 0 == false".
            let detail = self.format_assert_detail(condition, &condition_value);

            return Err(format!(
                "ASSERTION ERROR: {}\n  - {}",
                error_message, detail
            ));
        }

        Ok(Value::Void)
    }

    /// Produce the "why did this assertion fail" line. For infix
    /// comparisons we re-evaluate the operands to show their values;
    /// for anything else we just show the final value.
    fn format_assert_detail(&mut self, condition: &Node, final_value: &Value) -> String {
        if let Node::InfixExpression { left, operator, right, .. } = condition
            && matches!(operator.as_str(), "==" | "!=" | "<" | ">" | "<=" | ">=")
            && let (Ok(lv), Ok(rv)) = (self.eval(left), self.eval(right))
        {
            return format!(
                "condition {} {} {} was {}",
                lv, operator, rv, final_value
            );
        }
        format!("Condition evaluated to: {}", final_value)
    }
    
    fn eval_prefix_expression(&mut self, operator: &str, right: Value) -> RResult<Value> {
        match operator {
            "!" => self.eval_bang_operator_expression(right),
            "-" => self.eval_minus_prefix_operator_expression(right),
            _ => Err(format!("Unknown operator: {}{}", operator, right)),
        }
    }
    
    fn eval_bang_operator_expression(&mut self, right: Value) -> RResult<Value> {
        match right {
            Value::Bool(b) => Ok(Value::Bool(!b)),
            Value::Int(0) => Ok(Value::Bool(true)),
            Value::Int(_) => Ok(Value::Bool(false)),
            Value::Float(0.0) => Ok(Value::Bool(true)),
            Value::Float(_) => Ok(Value::Bool(false)),
            Value::String(s) if s.is_empty() => Ok(Value::Bool(true)),
            Value::String(_) => Ok(Value::Bool(false)),
            _ => Ok(Value::Bool(false)),
        }
    }
    
    fn eval_minus_prefix_operator_expression(&mut self, right: Value) -> RResult<Value> {
        match right {
            Value::Int(i) => Ok(Value::Int(-i)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(format!("Unknown operator: -{}", right)),
        }
    }
    
    fn eval_infix_expression(&mut self, operator: &str, left: Value, right: Value) -> RResult<Value> {
        // String + <primitive> coercion (RES-008): when `+` has a string on
        // either side and the other side is a primitive (int / float / bool),
        // coerce the primitive to its textual form and concatenate. This only
        // applies to `+` — other operators keep their strict behavior.
        if operator == "+"
            && (matches!(left, Value::String(_)) || matches!(right, Value::String(_)))
            && let (Some(ls), Some(rs)) = (
                stringify_for_concat(&left),
                stringify_for_concat(&right),
            )
        {
            return Ok(Value::String(format!("{ls}{rs}")));
        }

        // Array concat: `[1,2] + [3]` → `[1,2,3]`. Only for `+`.
        if operator == "+"
            && let (Value::Array(mut l), Value::Array(r)) = (left.clone(), right.clone())
        {
            l.extend(r);
            return Ok(Value::Array(l));
        }

        match (left.clone(), right.clone()) {
            (Value::Int(l), Value::Int(r)) => self.eval_integer_infix_expression(operator, l, r),
            (Value::Float(l), Value::Float(r)) => self.eval_float_infix_expression(operator, l, r),
            // RES-130: no implicit int ↔ float coercion at runtime.
            // The typechecker rejects this before eval when the
            // program is typechecked; the runtime guard below
            // catches the same shape for programs bypassing
            // `--typecheck`.
            (Value::Int(_), Value::Float(_)) | (Value::Float(_), Value::Int(_)) => {
                Err(format!(
                    "Cannot apply '{}' to int and float — Resilient does not implicitly coerce between numeric types. Use `to_float(x)` or `to_int(x)` explicitly.",
                    operator
                ))
            }
            (Value::String(l), Value::String(r)) => self.eval_string_infix_expression(operator, l, r),
            (Value::Bool(l), Value::Bool(r)) => self.eval_boolean_infix_expression(operator, l, r),
            _ => Err(format!("Type mismatch: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_integer_infix_expression(&mut self, operator: &str, left: i64, right: i64) -> RResult<Value> {
        match operator {
            "+" => Ok(Value::Int(left + right)),
            "-" => Ok(Value::Int(left - right)),
            "*" => Ok(Value::Int(left * right)),
            "/" => {
                if right == 0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Int(left / right))
                }
            },
            "%" => {
                if right == 0 {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(Value::Int(left % right))
                }
            },
            "&" => Ok(Value::Int(left & right)),
            "|" => Ok(Value::Int(left | right)),
            "^" => Ok(Value::Int(left ^ right)),
            "<<" => {
                if !(0..64).contains(&right) {
                    Err(format!("shift amount out of range: {}", right))
                } else {
                    Ok(Value::Int(left << right))
                }
            },
            ">>" => {
                if !(0..64).contains(&right) {
                    Err(format!("shift amount out of range: {}", right))
                } else {
                    Ok(Value::Int(left >> right))
                }
            },
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_float_infix_expression(&mut self, operator: &str, left: f64, right: f64) -> RResult<Value> {
        match operator {
            "+" => Ok(Value::Float(left + right)),
            "-" => Ok(Value::Float(left - right)),
            "*" => Ok(Value::Float(left * right)),
            "/" => {
                if right == 0.0 {
                    Err("Division by zero".to_string())
                } else {
                    Ok(Value::Float(left / right))
                }
            },
            "%" => {
                if right == 0.0 {
                    Err("Modulo by zero".to_string())
                } else {
                    Ok(Value::Float(left % right))
                }
            },
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_string_infix_expression(&mut self, operator: &str, left: String, right: String) -> RResult<Value> {
        // Lexicographic comparison for <, >, <=, >= matches the standard
        // behavior users expect from strings in most languages.
        match operator {
            "+" => Ok(Value::String(format!("{}{}", left, right))),
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "<" => Ok(Value::Bool(left < right)),
            ">" => Ok(Value::Bool(left > right)),
            "<=" => Ok(Value::Bool(left <= right)),
            ">=" => Ok(Value::Bool(left >= right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_boolean_infix_expression(&mut self, operator: &str, left: bool, right: bool) -> RResult<Value> {
        match operator {
            "==" => Ok(Value::Bool(left == right)),
            "!=" => Ok(Value::Bool(left != right)),
            "&&" => Ok(Value::Bool(left && right)),
            "||" => Ok(Value::Bool(left || right)),
            _ => Err(format!("Unknown operator: {} {} {}", left, operator, right)),
        }
    }
    
    fn eval_expressions(&mut self, expressions: &[Node]) -> RResult<Vec<Value>> {
        let mut result = Vec::new();
        
        for expr in expressions {
            let value = self.eval(expr)?;
            result.push(value);
        }
        
        Ok(result)
    }
    
    fn apply_function(&mut self, func: Value, args: Vec<Value>) -> RResult<Value> {
        match func {
            Value::Function { parameters, body, env, requires, ensures, name } => {
                // RES-050: env.clone() is now an Rc bump, not a deep
                // copy. The self-bind hack from c58c4b1 is gone — the
                // captured env IS the same RefCell that gets the
                // function's name rebound, so recursion works through
                // shared mutation.
                let extended_env = Environment::new_enclosed(env);

                for (i, (_, param_name)) in parameters.iter().enumerate() {
                    if i < args.len() {
                        extended_env.set(param_name.clone(), args[i].clone());
                    }
                }

                let mut interpreter = Interpreter {
                    env: extended_env,
                    statics: self.statics.clone(),
                    proven_fns: self.proven_fns.clone(),
                };

                // RES-035: check each `requires` clause BEFORE running
                // the body. Parameters are already in scope; anything
                // else (e.g. `static` bindings, closed-over vars) is
                // reachable just like inside the body.
                for clause in &requires {
                    let v = interpreter.eval(clause)?;
                    if !interpreter.is_truthy(&v) {
                        return Err(format!(
                            "Contract violation in fn {}: requires {} failed",
                            name,
                            format_contract_expr(clause)
                        ));
                    }
                }

                let body_result = interpreter.eval(&body)?;
                let return_value = if let Value::Return(v) = body_result {
                    *v
                } else {
                    body_result
                };

                // RES-035: check each `ensures` clause AFTER, with the
                // special identifier `result` bound to the return value.
                if !ensures.is_empty() {
                    interpreter
                        .env
                        .set("result".to_string(), return_value.clone());
                    for clause in &ensures {
                        let v = interpreter.eval(clause)?;
                        if !interpreter.is_truthy(&v) {
                            return Err(format!(
                                "Contract violation in fn {}: ensures {} failed (result = {})",
                                name,
                                format_contract_expr(clause),
                                return_value
                            ));
                        }
                    }
                }

                Ok(return_value)
            }
            Value::Builtin { func, .. } => func(&args),
            _ => Err(format!("Not a function: {}", func)),
        }
    }
    
    /// RES-039: test a pattern against a value. On match, returns
    /// `Some(binding)` where binding is `Some((name, value))` for an
    /// identifier pattern or `None` otherwise. On no match, returns `None`.
    #[allow(clippy::type_complexity)]
    fn match_pattern(
        &mut self,
        pattern: &Pattern,
        value: &Value,
    ) -> RResult<Option<Option<(String, Value)>>> {
        match pattern {
            Pattern::Wildcard => Ok(Some(None)),
            Pattern::Identifier(name) => Ok(Some(Some((name.clone(), value.clone())))),
            Pattern::Literal(node) => {
                let pat_val = self.eval(node)?;
                // RES-130: no int ↔ float coercion for literal-
                // pattern matching either. Different numeric types
                // just don't match — same policy as arithmetic.
                let is_equal = match (&pat_val, value) {
                    (Value::Int(a), Value::Int(b)) => a == b,
                    (Value::Float(a), Value::Float(b)) => a == b,
                    (Value::String(a), Value::String(b)) => a == b,
                    (Value::Bool(a), Value::Bool(b)) => a == b,
                    _ => false,
                };
                Ok(if is_equal { Some(None) } else { None })
            }
            // RES-160: first-match wins. The typechecker's
            // same-bindings-across-branches check (see
            // `pattern_bindings` in typechecker.rs) guarantees
            // the returned binding shape is consistent, so the
            // caller doesn't need to reconcile.
            Pattern::Or(branches) => {
                for b in branches {
                    if let Some(binding) = self.match_pattern(b, value)? {
                        return Ok(Some(binding));
                    }
                }
                Ok(None)
            }
        }
    }

    fn is_truthy(&self, value: &Value) -> bool {
        match value {
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }
}

// REPL for interactive evaluation.
// Kept as a reference implementation; the actual REPL used is `repl::EnhancedREPL`.
#[allow(dead_code)]
fn start_repl() -> RustylineResult<()> {
    let mut interpreter = Interpreter::new();
    let mut rl = DefaultEditor::new()?;
    let mut type_check_enabled = false;
    
    // Load history if available
    let history_path = match env::var("HOME") {
        Ok(home) => Path::new(&home).join(".resilient_history"),
        Err(_) => Path::new(".resilient_history").to_path_buf(),
    };
    
    if history_path.exists()
        && let Err(err) = rl.load_history(&history_path)
    {
        eprintln!("Error loading history: {}", err);
    }

    println!("Resilient Programming Language REPL (v0.1.0)");
    println!("Type 'exit' to quit, 'help' for command list");
    
    loop {
        let prompt = if type_check_enabled {
            ">> [typecheck] "
        } else {
            ">> "
        };
        
        let readline = rl.readline(prompt);
        
        match readline {
            Ok(line) => {
                let input = line.trim();
                
                // Skip empty lines
                if input.is_empty() {
                    continue;
                }
                
                // Add to history
                rl.add_history_entry(input)?;
                
                // Handle special commands
                match input {
                    "exit" | "quit" => break,
                    "help" => {
                        println!("Available commands:");
                        println!("  help       - Show this help message");
                        println!("  exit       - Exit the REPL");
                        println!("  clear      - Clear the screen");
                        println!("  examples   - Show example code snippets");
                        println!("  typecheck  - Toggle type checking (currently {})", 
                                 if type_check_enabled { "enabled" } else { "disabled" });
                        continue;
                    },
                    "clear" => {
                        print!("\x1B[2J\x1B[1;1H"); // ANSI escape code to clear screen
                        io::stdout().flush().unwrap();
                        continue;
                    },
                    "typecheck" => {
                        type_check_enabled = !type_check_enabled;
                        println!("Type checking {}", 
                                 if type_check_enabled { "enabled" } else { "disabled" });
                        continue;
                    },
                    "examples" => {
                        println!("Example code snippets:");
                        println!("\n1. Basic variable and function:");
                        println!("let x = 42;");
                        println!("fn add(int a, int b) {{ return a + b; }}");
                        println!("add(x, 10);");
                        
                        println!("\n2. Live block example:");
                        println!("live {{");
                        println!("  let result = 100 / 0; // This would normally crash");
                        println!("  println(\"Result: \" + result);");
                        println!("}}");
                        
                        println!("\n3. Assertion example:");
                        println!("let age = 25;");
                        println!("assert(age >= 18, \"Must be an adult\");");
                        println!("println(\"Access granted\");");
                        continue;
                    },
                    _ => {}
                }
                
                // Parse the input
                let lexer = Lexer::new(input.to_string());
                let mut parser = Parser::new(lexer);
                let program = parser.parse_program();
                
                // Skip evaluation if any parser errors were recorded
                if !parser.errors.is_empty() {
                    continue;
                }
                
                // Run type checker if enabled
                if type_check_enabled {
                    match typechecker::TypeChecker::new().check_program(&program) {
                        Ok(_) => println!("\x1B[32mType check passed\x1B[0m"), // Green text
                        Err(e) => {
                            eprintln!("\x1B[31mType error: {}\x1B[0m", e); // Red text
                            continue; // Skip execution if type checking fails
                        }
                    }
                }
                
                // Evaluate the input
                match interpreter.eval(&program) {
                    Ok(value) => {
                        if !matches!(value, Value::Void) {
                            println!("{}", value);
                        }
                    },
                    Err(error) => {
                        eprintln!("\x1B[31mError: {}\x1B[0m", error); // Red error text
                    }
                }
            },
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            },
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            },
            Err(err) => {
                eprintln!("Error: {}", err);
                break;
            }
        }
    }
    
    // Save history
    if let Err(err) = rl.save_history(&history_path) {
        eprintln!("Error saving history: {}", err);
    }
    
    Ok(())
}

/// RES-116: tag a raw interpreter error string with the enclosing
/// statement's `line:col:` prefix so `execute_file` can reformat it
/// into the full `filename:line:col: Runtime error: <msg>` shape.
///
/// Errors already carrying a `line:col:` prefix pass through
/// untouched — this happens when an inner call (another statement
/// executed via the builtin path or a nested block) already did the
/// decoration and the outer statement shouldn't double-wrap.
fn decorate_runtime_error(msg: String, span: &span::Span) -> String {
    if has_line_col_prefix(&msg) {
        msg
    } else {
        format!("{}:{}: {}", span.start.line, span.start.column, msg)
    }
}

/// True if `msg` starts with `<digits>:<digits>:` — the sentinel shape
/// produced by `decorate_runtime_error` and the typechecker's
/// line:col: prefix (RES-080).
fn has_line_col_prefix(msg: &str) -> bool {
    let mut it = msg.chars();
    let mut saw_digit = false;
    for c in it.by_ref() {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if c == ':' && saw_digit {
            break;
        } else {
            return false;
        }
    }
    if !saw_digit {
        return false;
    }
    // Now expect another digits:... segment
    saw_digit = false;
    for c in it {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else {
            return c == ':' && saw_digit;
        }
    }
    false
}

/// RES-116: format an interpreter runtime error for the driver's
/// stderr channel. Errors decorated by `decorate_runtime_error` get
/// the full `filename:line:col: Runtime error: <msg>` prefix; un-
/// decorated errors (e.g. pre-statement-evaluation issues that never
/// reached `eval_program`) fall back to a bare `Runtime error: <msg>`.
///
/// RES-117: when `src` is provided, appends a caret diagnostic from
/// `diag::format_diagnostic_from_line_col` below the header line so
/// the offending source position is visually underlined.
fn format_interpreter_error(filename: &str, err: &str) -> String {
    if has_line_col_prefix(err) {
        let header = format!("{}:{}", filename, err.replacen(": ", ": Runtime error: ", 1));
        header
    } else {
        format!("Runtime error: {}", err)
    }
}

/// RES-117: enrich a `line:col:` prefixed error string with a caret
/// underline pulled from `src`. Returns `err` unchanged when the
/// prefix can't be parsed. Callers supply `level` (e.g. `"Runtime
/// error"`, `"Parser error"`, `"Type error"`, `"VM runtime error"`)
/// and the helper handles the rest.
///
/// Handles both shapes the codebase uses today:
/// - `<line>:<col>: <msg>` (parser errors)
/// - `<path>:<line>:<col>: <msg>` (typechecker, decorated runtime)
///
/// If `msg` already starts with `<level>:` (the driver composed the
/// header with the level baked in), that inner duplicate is
/// stripped so the caret block doesn't read `Runtime error: Runtime
/// error: ...`.
fn render_with_caret(src: &str, err: &str, level: &str) -> String {
    let dedupe = |msg: &str| -> String {
        let trimmed = msg.trim();
        let prefix = format!("{}:", level);
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            rest.trim().to_string()
        } else {
            trimmed.to_string()
        }
    };

    // Try the bare `<line>:<col>: <msg>` form first.
    let mut it = err.splitn(3, ':');
    if let (Some(line_s), Some(col_s), Some(rest)) = (it.next(), it.next(), it.next())
        && let (Ok(line), Ok(col)) = (line_s.trim().parse::<usize>(), col_s.trim().parse::<usize>())
    {
        return format!(
            "{}\n{}",
            err,
            diag::format_diagnostic_from_line_col(src, line, col, level, &dedupe(rest))
        );
    }

    // Fall back to the `<path>:<line>:<col>: <msg>` shape — find the
    // FIRST colon after which `<uint>:<uint>:` follows.
    for (i, _) in err.match_indices(':') {
        let tail = &err[i + 1..];
        let mut parts = tail.splitn(3, ':');
        let ls = parts.next().unwrap_or("");
        let cs = parts.next().unwrap_or("");
        let msg = parts.next().unwrap_or("");
        if let (Ok(line), Ok(col)) = (ls.trim().parse::<usize>(), cs.trim().parse::<usize>()) {
            return format!(
                "{}\n{}",
                err,
                diag::format_diagnostic_from_line_col(src, line, col, level, &dedupe(msg))
            );
        }
    }
    err.to_string()
}

/// RES-112: scan `src` through the default routing (hand-rolled or
/// logos lexer, whichever the build has) and print one token per
/// line on stdout in the format
/// `<line>:<col>  <Kind>("<lexeme>")`, with the lexeme extracted
/// from the source using the token's span. Terminates at `Eof`.
///
/// The emitted format is driven by the token's `Debug` impl for
/// variant naming, so adding a new `Token` variant automatically
/// shows up here without a matching change.
fn dump_tokens_to_stdout(src: &str) {
    // Pre-index the source as chars so we can slice by the lexer's
    // char-offset `Span::{start.offset, end.offset}` regardless of
    // UTF-8 boundaries.
    let chars: Vec<char> = src.chars().collect();
    let mut lex = Lexer::new(src.to_string());
    loop {
        let (tok, span) = lex.next_token_with_span();
        let is_eof = matches!(tok, Token::Eof);
        let lexeme: String = if span.end.offset > span.start.offset
            && span.end.offset <= chars.len()
        {
            chars[span.start.offset..span.end.offset].iter().collect()
        } else {
            String::new()
        };
        // Escape newlines / quotes in the lexeme so block comments
        // and multi-line string literals stay readable.
        let lexeme = lexeme.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        println!(
            "{}:{}  {:?}(\"{}\")",
            span.start.line, span.start.column, tok, lexeme
        );
        if is_eof {
            break;
        }
    }
}

/// RES-073: shared parse helper. Returns the parsed program plus any
/// parser error strings collected along the way. Used by both the
/// driver and `imports::expand_uses`.
fn parse(src: &str) -> (Node, Vec<String>) {
    let lexer = Lexer::new(src.to_string());
    let mut parser = Parser::new(lexer);
    let program = parser.parse_program();
    let errs: Vec<String> = parser.errors.into_iter().map(|e| e.to_string()).collect();
    (program, errs)
}

/// RES-187: semantic-token type indices. Must match the legend
/// `lsp_server::SEMANTIC_TOKEN_TYPES` declares in its capability
/// advertisement. Keep in sync — the LSP spec encodes these as
/// indices into that legend, not names.
#[allow(dead_code)] // only used behind the `lsp` feature
pub(crate) mod sem_tok {
    pub const KEYWORD: u32 = 0;
    pub const FUNCTION: u32 = 1;
    pub const VARIABLE: u32 = 2;
    pub const PARAMETER: u32 = 3;
    pub const TYPE: u32 = 4;
    pub const STRING: u32 = 5;
    pub const NUMBER: u32 = 6;
    pub const COMMENT: u32 = 7;
    pub const OPERATOR: u32 = 8;

    pub const MOD_DECLARATION: u32 = 1 << 0;
    #[allow(dead_code)]
    pub const MOD_READONLY: u32 = 1 << 1;
}

/// RES-187: one semantic-token tuple before delta encoding.
/// Absolute (line, col) so we can sort before encoding to the
/// LSP wire format.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // only used behind the `lsp` feature
pub(crate) struct AbsSemToken {
    pub line: u32,
    pub col: u32,
    pub length: u32,
    pub ty: u32,
    pub modifiers: u32,
}

/// RES-187: compute semantic tokens for `src` and encode them as
/// the LSP delta-array format `[deltaLine, deltaStart, length,
/// tokenType, modifiers]*`. The result is what `SemanticTokens
/// { data, .. }` carries back to the client.
///
/// Approach: walk the lexer's token stream (runs in-process — we
/// keep the same lexer the compiler uses, so keyword / literal
/// lists can't drift). A tiny state machine handles the
/// keyword-context cases where the lexer alone can't distinguish
/// (identifier after `fn` → FUNCTION + DECLARATION; after
/// `struct`/`type` → TYPE + DECLARATION; after `let`/`static` →
/// VARIABLE + DECLARATION; after `new` → TYPE; default identifier
/// → VARIABLE). A separate source-text sweep picks up line and
/// block comments which the lexer discards.
///
/// Each tuple is absolute at collection time, then sorted by
/// (line, col) and delta-encoded in a final pass per the LSP
/// spec. Nothing here allocates per-token beyond the Vec itself;
/// large files should be fine.
#[allow(dead_code)] // only used behind the `lsp` feature
pub(crate) fn compute_semantic_tokens(src: &str) -> Vec<u32> {
    let tokens = collect_semantic_tokens(src);
    encode_semantic_tokens(&tokens)
}

/// RES-187: the "collect" half of compute_semantic_tokens.
/// Exposed separately so unit tests can assert on the absolute-
/// coordinate tuples without re-decoding the delta array.
#[allow(dead_code)]
pub(crate) fn collect_semantic_tokens(src: &str) -> Vec<AbsSemToken> {
    let mut out: Vec<AbsSemToken> = Vec::new();
    // Lexer-driven pass for keywords / literals / operators /
    // identifiers.
    let mut lex = Lexer::new(src.to_string());
    let mut prev_kw: Option<Token> = None;
    loop {
        let (tok, span) = lex.next_token_with_span();
        if matches!(tok, Token::Eof) {
            break;
        }
        if let Some(entry) = classify_lex_token(&tok, prev_kw.as_ref(), span) {
            out.push(entry);
        }
        // Track "previous keyword" for the next identifier's
        // context classification. We reset to None on tokens
        // that would break the context (e.g. a `,` between
        // params clears the `fn`-context so the NEXT identifier
        // after the open paren is treated as a parameter name,
        // not the function's own name).
        prev_kw = match &tok {
            Token::Function
            | Token::Struct
            | Token::Type
            | Token::New
            | Token::Let
            | Token::Static => Some(tok.clone()),
            _ => None,
        };
    }
    // Comment pass — the lexer discards these so we scan the
    // source text for `// ... \n` and `/* ... */`.
    out.extend(scan_comment_tokens(src));
    out
}

/// RES-187: map one lexer token to a semantic-token tuple,
/// given the most-recently-seen keyword context.
fn classify_lex_token(
    tok: &Token,
    prev_kw: Option<&Token>,
    span: span::Span,
) -> Option<AbsSemToken> {
    // LSP uses 0-indexed line/character.
    let line = span.start.line.saturating_sub(1) as u32;
    let col = span.start.column.saturating_sub(1) as u32;
    // Length in chars (RES-115: offsets are char-counts).
    let length = span.end.offset.saturating_sub(span.start.offset) as u32;

    let (ty, modifiers) = match tok {
        // Keywords.
        Token::Function | Token::Let | Token::Live | Token::Assert
        | Token::If | Token::Else | Token::Return | Token::Static
        | Token::While | Token::For | Token::In
        | Token::Requires | Token::Ensures | Token::Invariant
        | Token::Struct | Token::New | Token::Match | Token::Use
        | Token::Impl | Token::Type | Token::Default
        | Token::BoolLiteral(_) => (sem_tok::KEYWORD, 0),

        // Numeric / string / bytes literals.
        Token::IntLiteral(_) | Token::FloatLiteral(_) => (sem_tok::NUMBER, 0),
        Token::StringLiteral(_) | Token::BytesLiteral(_) => (sem_tok::STRING, 0),

        // Identifiers: context-dependent.
        Token::Identifier(_) => match prev_kw {
            Some(Token::Function) => (sem_tok::FUNCTION, sem_tok::MOD_DECLARATION),
            Some(Token::Struct) | Some(Token::Type) => {
                (sem_tok::TYPE, sem_tok::MOD_DECLARATION)
            }
            Some(Token::New) => (sem_tok::TYPE, 0),
            Some(Token::Let) | Some(Token::Static) => {
                (sem_tok::VARIABLE, sem_tok::MOD_DECLARATION)
            }
            _ => (sem_tok::VARIABLE, 0),
        },

        // Operators. Covers arithmetic, comparison, logical,
        // bitwise, assignment, prefix !, and the match-arrow
        // forms. Brackets / braces / parens / semicolons /
        // commas are delimiters not highlighted as operators in
        // any standard LSP legend, so they get no token.
        Token::Plus | Token::Minus | Token::Multiply | Token::Divide
        | Token::Modulo | Token::Assign | Token::Equal | Token::NotEqual
        | Token::And | Token::Or | Token::BitAnd | Token::BitOr
        | Token::BitXor | Token::ShiftLeft | Token::ShiftRight
        | Token::Greater | Token::Less | Token::GreaterEqual | Token::LessEqual
        | Token::Bang | Token::Dot | Token::FatArrow | Token::Arrow
        | Token::Question => (sem_tok::OPERATOR, 0),

        // Delimiters + internal tokens: no semantic highlight.
        _ => return None,
    };
    // Zero-length tokens aren't useful and some clients reject
    // them; skip.
    if length == 0 {
        return None;
    }
    Some(AbsSemToken { line, col, length, ty, modifiers })
}

/// RES-187: scan `src` for `// ... \n` line comments and
/// `/* ... */` block comments, emitting one token per comment.
/// Walks char-by-char; only allocates the output vec. Handles
/// nested-less block comments — our lexer doesn't support nested
/// either.
fn scan_comment_tokens(src: &str) -> Vec<AbsSemToken> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut line: u32 = 0;
    let mut col: u32 = 0;
    let mut i = 0;
    while i < chars.len() {
        // Line comment?
        if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
            let start_line = line;
            let start_col = col;
            let mut j = i;
            while j < chars.len() && chars[j] != '\n' {
                j += 1;
            }
            out.push(AbsSemToken {
                line: start_line,
                col: start_col,
                length: (j - i) as u32,
                ty: sem_tok::COMMENT,
                modifiers: 0,
            });
            col += (j - i) as u32;
            i = j;
            continue;
        }
        // Block comment?
        if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '*' {
            let start_line = line;
            let start_col = col;
            let mut j = i + 2;
            // Each block comment produces ONE token per line it
            // spans — most clients render a single token as a
            // unit, but if a block comment spans multiple lines
            // we emit per-line tokens so the delta-array stays
            // legal (tokens must not span line boundaries per
            // the LSP spec). Walk tracking current line/col.
            let mut cur_line = start_line;
            let mut cur_col = start_col;
            let mut seg_start_col = cur_col;
            while j + 1 < chars.len() && !(chars[j] == '*' && chars[j + 1] == '/') {
                if chars[j] == '\n' {
                    // Flush the segment on this line.
                    let seg_chars = j - i;
                    let length = seg_chars as u32 - (seg_start_col - start_col);
                    // But we want per-line tokens: split into
                    // [cur_line, seg_start_col .. cur_col].
                    // cur_col has been tracking; use it as the
                    // end of segment.
                    let seg_len = cur_col - seg_start_col;
                    if seg_len > 0 {
                        out.push(AbsSemToken {
                            line: cur_line,
                            col: seg_start_col,
                            length: seg_len,
                            ty: sem_tok::COMMENT,
                            modifiers: 0,
                        });
                    }
                    cur_line += 1;
                    cur_col = 0;
                    seg_start_col = 0;
                    let _ = length; // suppress unused warning in some configs
                } else {
                    cur_col += 1;
                }
                j += 1;
            }
            // Closing `*/` (if present).
            if j + 1 < chars.len() && chars[j] == '*' && chars[j + 1] == '/' {
                cur_col += 2;
                j += 2;
            }
            // Flush the final segment.
            let seg_len = cur_col.saturating_sub(seg_start_col);
            if seg_len > 0 {
                out.push(AbsSemToken {
                    line: cur_line,
                    col: seg_start_col,
                    length: seg_len,
                    ty: sem_tok::COMMENT,
                    modifiers: 0,
                });
            }
            // Sync outer line/col/i to where we ended up.
            line = cur_line;
            col = cur_col;
            i = j;
            continue;
        }
        // Advance (and track newlines).
        if chars[i] == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
        i += 1;
    }
    out
}

/// RES-187: encode a vec of absolute-coord semantic tokens into
/// the LSP delta-array wire format `[dLine, dStart, length,
/// type, modifiers]` per-token, sorted by (line, col) and
/// non-overlapping. Overlapping tokens would be rejected by
/// strict clients; our emitters don't produce overlaps in
/// practice (lex tokens are disjoint, comments are in their
/// own spans), but sort-and-dedupe at the end is cheap
/// insurance.
#[allow(dead_code)]
pub(crate) fn encode_semantic_tokens(tokens: &[AbsSemToken]) -> Vec<u32> {
    let mut sorted: Vec<AbsSemToken> = tokens.to_vec();
    sorted.sort_by(|a, b| a.line.cmp(&b.line).then(a.col.cmp(&b.col)));

    let mut out = Vec::with_capacity(sorted.len() * 5);
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    for t in &sorted {
        let d_line = t.line - prev_line;
        let d_start = if d_line == 0 { t.col - prev_col } else { t.col };
        out.extend_from_slice(&[d_line, d_start, t.length, t.ty, t.modifiers]);
        prev_line = t.line;
        prev_col = t.col;
    }
    out
}

/// RES-071: writes accumulated SMT-LIB2 certificates to `dir`. One file
/// per discharged obligation: `{fn_name}__{kind}__{idx}.smt2`. Returns
/// the count written for the audit summary.
fn emit_certificates(
    certificates: &[typechecker::CapturedCertificate],
    dir: &Path,
) -> RResult<usize> {
    fs::create_dir_all(dir).map_err(|e| {
        format!("could not create certificate directory {}: {}", dir.display(), e)
    })?;
    for cert in certificates {
        // Sanitize fn_name: only [A-Za-z0-9_] survives, others become '_'.
        let safe: String = cert.fn_name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .collect();
        let path = dir.join(format!("{}__{}__{}.smt2", safe, cert.kind, cert.idx));
        fs::write(&path, &cert.smt2)
            .map_err(|e| format!("could not write {}: {}", path.display(), e))?;
    }
    Ok(certificates.len())
}

// Execute a Resilient source file
fn execute_file(
    filename: &str,
    type_check: bool,
    audit: bool,
    emit_cert_dir: Option<&Path>,
    use_vm: bool,
    use_jit: bool,
    verifier_timeout_ms: u32,
) -> RResult<()> {
    let contents = fs::read_to_string(filename)
        .map_err(|e| format!("Error reading file: {}", e))?;

    let lexer = Lexer::new(contents.clone());
    let mut parser = Parser::new(lexer);
    let mut program = parser.parse_program();

    // Check for parser errors (already printed at the point they occurred).
    //
    // RES-117: print each collected error again with a caret
    // underline from the source. The original `record_error`
    // eprintln still fires inline (it's the low-latency path users
    // see while the parser is still scanning); this follow-up
    // dump gives them the source-context block. We keep the
    // original emission intact to minimise surgery on the parser.
    if !parser.errors.is_empty() {
        for e in &parser.errors {
            eprintln!("{}", render_with_caret(&contents, e, "Parser error"));
        }
        return Err(format!("Failed to parse program: {} parser error(s)", parser.errors.len()));
    }

    // RES-073: resolve `use` imports before typecheck / interpret.
    let base_dir = Path::new(filename)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let mut loaded: HashSet<PathBuf> = HashSet::new();
    // Seed with the canonicalized current file so circular `use`s are
    // detected if a re-import points back at us.
    if let Ok(canon) = fs::canonicalize(filename) {
        loaded.insert(canon);
    }
    if let Err(e) = imports::expand_uses(&mut program, &base_dir, &mut loaded) {
        return Err(format!("Import error: {}", e));
    }

    // Type checking if enabled. --audit and --emit-certificate both
    // imply --typecheck (no point running them without it).
    let want_typecheck = type_check || audit || emit_cert_dir.is_some();
    let mut proven_fns: HashSet<String> = HashSet::new();
    if want_typecheck {
        println!("Running type checker...");
        let mut tc = typechecker::TypeChecker::new()
            // RES-137: apply the driver's --verifier-timeout-ms
            // value. A fresh `TypeChecker::new()` defaults to 5000;
            // passing the CLI value through keeps the flag
            // meaningful on the `--typecheck` / `--audit` paths.
            .with_verifier_timeout_ms(verifier_timeout_ms);
        // RES-080: pass the source filename so per-statement errors
        // are prefixed with `<file>:<line>:<col>:`.
        match tc.check_program_with_source(&program, filename) {
            Ok(_) => println!("\x1B[32mType check passed\x1B[0m"),
            Err(e) => {
                eprintln!("\x1B[31mType error: {}\x1B[0m", e);
                // RES-117: add a caret diagnostic beneath the
                // ANSI-red header so the offending source position
                // is visually underlined.
                eprintln!("{}", render_with_caret(&contents, &e, "Type error"));
                return Err(format!("Type check failed: {}", e));
            }
        }
        // RES-068: harvest the set of fns whose contracts the
        // typechecker fully discharged, so the interpreter can skip
        // their runtime requires checks.
        proven_fns = tc.stats.fully_provable_fns();
        if audit {
            print_verification_audit(&tc.stats);
        }
        // RES-071: dump SMT-LIB2 certificates for every Z3-discharged
        // obligation so a downstream consumer can re-verify with
        // stock Z3 and confirm the proof without trusting our binary.
        if let Some(dir) = emit_cert_dir {
            let n = emit_certificates(&tc.certificates, dir)?;
            println!(
                "\x1B[36mWrote {} verification certificate(s) to {}\x1B[0m",
                n,
                dir.display()
            );
        }
    }

    if use_jit {
        // RES-072 Phase A: Cranelift JIT path. Stub today; RES-096+
        // will add real AST lowering. Surfaces a clean error
        // through the same `<file>: ...` shape as the VM (RES-095)
        // so the user knows the JIT isn't implemented yet without
        // a panic or opaque message.
        #[cfg(feature = "jit")]
        {
            let result = jit_backend::run(&program)
                .map_err(|e| format!("{}: {}", filename, e))?;
            println!("{}", result);
            return Ok(());
        }
        #[cfg(not(feature = "jit"))]
        {
            return Err(
                "--jit requires the `jit` feature. Rebuild with:\n  \
                 cargo build --features jit"
                    .to_string(),
            );
        }
    }

    if use_vm {
        // RES-076 + RES-081: bytecode VM path. Compile the AST into
        // a Program (main chunk + function table), run it, print the
        // resulting value (mirroring the tree walker's behavior for
        // non-Void results).
        let prog = compiler::compile(&program)
            .map_err(|e| format!("VM compile error: {}", e))?;
        let result = vm::run(&prog).map_err(|e| {
            // RES-095: mirror the typechecker's `<file>:<line>:` shape
            // so VM runtime errors are editor-clickable when the
            // wrapper carries a source line. Other variants fall back
            // to the bare Display form.
            //
            // RES-117: `AtLine` carries line but not column — treat
            // as column 1 for the caret renderer. The caret still
            // points at the offending line; precise column info
            // would need RES-091 to upgrade from line-only to a
            // full Span (tracked there, not here).
            if let vm::VmError::AtLine { line, kind } = &e {
                let header = format!("{}:{}: VM runtime error: {}", filename, line, kind);
                let caret = diag::format_diagnostic_from_line_col(
                    &contents,
                    *line as usize,
                    1,
                    "VM runtime error",
                    &kind.to_string(),
                );
                format!("{}\n{}", header, caret)
            } else {
                format!("VM runtime error: {}", e)
            }
        })?;
        if !matches!(result, Value::Void) {
            println!("{}", result);
        }
        return Ok(());
    }

    let mut interpreter = Interpreter::new().with_proven_fns(proven_fns);
    // RES-116: runtime errors from the tree-walker now carry a
    // `line:col:` prefix (applied in `eval_program`). Reshape that
    // here into `filename:line:col: Runtime error: <msg>` so the
    // driver's output matches the VM's RES-091 shape and is
    // editor-clickable. Un-decorated errors fall back to the older
    // bare `Runtime error: <msg>` format.
    //
    // RES-117: also attach a caret diagnostic beneath the header
    // so the offending source line is visually underlined.
    interpreter.eval(&program).map_err(|e| {
        let header = format_interpreter_error(filename, &e);
        if has_line_col_prefix(&e) {
            render_with_caret(&contents, &header, "Runtime error")
        } else {
            header
        }
    })?;

    Ok(())
}

/// RES-066: print a structured verification report after a successful
/// typecheck. Tells the user exactly what the static verifier
/// discharged vs deferred to runtime.
fn print_verification_audit(stats: &typechecker::VerificationStats) {
    let total_callsite =
        stats.requires_discharged_at_compile + stats.requires_left_for_runtime;
    println!();
    println!("\x1B[36m--- Verification Audit ---\x1B[0m");
    println!(
        "  contract decls (tautologies discharged): \x1B[32m{}\x1B[0m",
        stats.requires_tautology
    );
    println!(
        "  contracted call sites visited:           \x1B[36m{}\x1B[0m",
        stats.contracted_call_sites
    );
    println!(
        "  call-site requires discharged statically: \x1B[32m{} / {}\x1B[0m",
        stats.requires_discharged_at_compile, total_callsite
    );
    if stats.requires_discharged_by_z3 > 0 {
        println!(
            "    of which proven by Z3 (SMT):            \x1B[35m{}\x1B[0m",
            stats.requires_discharged_by_z3
        );
    }
    // RES-137: timeouts sit alongside the runtime-retained
    // counter — the yellow bar of "we tried and gave up." Only
    // printed when non-zero so the common case stays tidy.
    if stats.verifier_timeouts > 0 {
        println!(
            "    of which timed out:                     \x1B[33m{}\x1B[0m",
            stats.verifier_timeouts
        );
    }
    println!(
        "  call-site requires left for runtime:      \x1B[33m{} / {}\x1B[0m",
        stats.requires_left_for_runtime, total_callsite
    );
    if total_callsite > 0 {
        let pct = (stats.requires_discharged_at_compile as f64 / total_callsite as f64) * 100.0;
        println!("  static coverage:                          \x1B[36m{:.0}%\x1B[0m", pct);
    }
}

// Example programs

/// RES-205: `resilient pkg <sub>` subcommand dispatcher. Runs
/// before the file-execution arg parser so the `pkg` verb doesn't
/// have to fight the existing flag grammar. Returns `Some(code)`
/// with the exit status if a `pkg` subcommand matched (and the
/// caller should exit); `None` if no `pkg` verb was seen and main
/// should fall through to its normal flow.
fn dispatch_pkg_subcommand(args: &[String]) -> Option<i32> {
    if args.get(1).map(|s| s.as_str()) != Some("pkg") {
        return None;
    }
    match args.get(2).map(|s| s.as_str()) {
        Some("init") => {
            // `pkg init <name>` — the only subcommand for now.
            // Future `pkg add`, `pkg build`, etc. will branch here.
            let name = match args.get(3) {
                Some(n) => n.as_str(),
                None => {
                    eprintln!(
                        "Error: {}",
                        pkg_init::PkgInitError::MissingName
                    );
                    return Some(2);
                }
            };
            let cwd = match env::current_dir() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error: could not read current directory: {}", e);
                    return Some(2);
                }
            };
            match pkg_init::scaffold_in(&cwd, name) {
                Ok(scaffold) => {
                    println!(
                        "Created {} at {}",
                        name,
                        scaffold.root.display()
                    );
                    for p in &scaffold.wrote {
                        println!("  wrote {}", p.display());
                    }
                    println!("\nNext steps:");
                    println!("  cd {}", name);
                    println!("  resilient src/main.rs");
                    Some(0)
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    Some(1)
                }
            }
        }
        Some(other) => {
            eprintln!(
                "Error: unknown pkg subcommand `{}`. Known: init",
                other
            );
            Some(2)
        }
        None => {
            eprintln!("Error: `resilient pkg` requires a subcommand. Known: init");
            Some(2)
        }
    }
}

fn main() {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();

    // RES-205: intercept `pkg` subcommands before the normal flow.
    // Exits directly on handled verbs so the rest of main stays
    // focused on the compiler driver.
    if let Some(code) = dispatch_pkg_subcommand(&args) {
        std::process::exit(code);
    }

    let mut type_check = false;
    let mut audit = false;
    let mut emit_cert_dir: Option<PathBuf> = None;
    let mut examples_dir: Option<PathBuf> = None;
    let mut use_vm = false;
    let mut use_jit = false;
    let mut lsp_mode = false;
    // RES-112: --dump-tokens prints the lexer output and exits, so
    // lexer regressions are inspectable without editing source.
    let mut dump_tokens = false;
    // RES-173: --dump-chunks compiles the program and prints a
    // human-readable VM disassembly. Reflects RES-172 peephole
    // results because the compiler runs peephole before the
    // disassembler sees the chunks.
    let mut dump_chunks = false;
    // RES-174: --jit-cache-stats prints the process-wide JIT AST-
    // hash cache counters (hits/misses/compiles) to stderr on
    // exit. Stats come from `jit_backend::cache_stats()` which
    // reads relaxed-atomic counters updated by each `run()`.
    let mut jit_cache_stats = false;
    // RES-137: per-query Z3 solver timeout in milliseconds. 0 means
    // "no timeout". Default 5000 matches the ticket's recommendation.
    let mut verifier_timeout_ms: u32 = 5000;
    // RES-150: `--seed <u64>` pins the RNG seed for determinism.
    // `None` → fall back to clock-derived seed at startup.
    let mut seed_override: Option<u64> = None;
    let mut filename = "";

    // Simple argument parsing
    if args.len() > 1 {
        let mut i = 1;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--typecheck" || arg == "-t" {
                type_check = true;
            } else if arg == "--audit" {
                audit = true;
            } else if arg == "--emit-certificate" {
                // RES-071: --emit-certificate <DIR>
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --emit-certificate requires a directory argument");
                    std::process::exit(2);
                }
                emit_cert_dir = Some(PathBuf::from(&args[i]));
            } else if let Some(dir) = arg.strip_prefix("--emit-certificate=") {
                emit_cert_dir = Some(PathBuf::from(dir));
            } else if arg == "--vm" {
                // RES-076: route through the bytecode VM instead of
                // the tree-walking interpreter.
                use_vm = true;
            } else if arg == "--jit" {
                // RES-072: route through the Cranelift JIT backend.
                // Phase A is a stub — RES-096+ adds real lowering.
                use_jit = true;
            } else if arg == "--lsp" {
                // RES-074: start the Language Server on stdio. Only
                // functional when built with `--features lsp`; the
                // non-feature path prints a helpful message and exits.
                lsp_mode = true;
            } else if arg == "--dump-tokens" {
                // RES-112: print the lexer's token stream and exit.
                // Accepts both the hand-rolled scanner (default) and
                // the logos-lexer feature path — both go through
                // `Lexer::new` + `next_token_with_span`.
                dump_tokens = true;
            } else if arg == "--dump-chunks" {
                // RES-173: compile the program to bytecode and print
                // a human-readable disassembly (RES-172 peephole
                // included). Exits after the dump — mutually
                // exclusive with `--lsp` / `--dump-tokens`.
                dump_chunks = true;
            } else if arg == "--jit-cache-stats" {
                // RES-174: print the JIT cache's cumulative
                // (hits / misses / compiles) on exit.
                jit_cache_stats = true;
            } else if arg == "--verifier-timeout-ms" {
                // RES-137: --verifier-timeout-ms <N> overrides the
                // per-Z3-query budget. `0` disables the timeout.
                i += 1;
                if i >= args.len() {
                    eprintln!(
                        "Error: --verifier-timeout-ms requires a positive integer argument (ms)"
                    );
                    std::process::exit(2);
                }
                verifier_timeout_ms = args[i].parse().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: --verifier-timeout-ms expects a u32, got {:?}",
                        args[i]
                    );
                    std::process::exit(2);
                });
            } else if let Some(val) = arg.strip_prefix("--verifier-timeout-ms=") {
                verifier_timeout_ms = val.parse().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: --verifier-timeout-ms expects a u32, got {:?}",
                        val
                    );
                    std::process::exit(2);
                });
            } else if arg == "--seed" {
                // RES-150: `--seed <u64>` pins the SplitMix64 PRNG.
                // Reproducible runs: same seed → same sequence.
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --seed requires a u64 argument");
                    std::process::exit(2);
                }
                seed_override = Some(args[i].parse().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: --seed expects a u64, got {:?}",
                        args[i]
                    );
                    std::process::exit(2);
                }));
            } else if let Some(val) = arg.strip_prefix("--seed=") {
                seed_override = Some(val.parse().unwrap_or_else(|_| {
                    eprintln!(
                        "Error: --seed expects a u64, got {:?}",
                        val
                    );
                    std::process::exit(2);
                }));
            } else if arg == "--examples-dir" {
                // RES-026: --examples-dir <DIR> for the REPL's
                // `examples` command.
                i += 1;
                if i >= args.len() {
                    eprintln!("Error: --examples-dir requires a directory argument");
                    std::process::exit(2);
                }
                examples_dir = Some(PathBuf::from(&args[i]));
            } else if let Some(dir) = arg.strip_prefix("--examples-dir=") {
                examples_dir = Some(PathBuf::from(dir));
            } else {
                filename = arg;
            }
            i += 1;
        }

        // RES-112: --dump-tokens is mutually exclusive with --lsp
        // (both are terminal modes that don't want a file arg the
        // other way). Emit a clean error if the user combined them.
        if dump_tokens && lsp_mode {
            eprintln!("Error: --dump-tokens and --lsp are mutually exclusive");
            std::process::exit(2);
        }
        // RES-173: --dump-chunks mutually exclusive with the other
        // terminal modes for the same reason.
        if dump_chunks && (lsp_mode || dump_tokens) {
            eprintln!("Error: --dump-chunks and --dump-tokens/--lsp are mutually exclusive");
            std::process::exit(2);
        }

        // RES-150: install the RNG seed before any user program
        // can pull from it. `--seed <N>` pins the sequence
        // (silently, since the user asked for reproducibility);
        // otherwise we derive from the monotonic clock and echo
        // the chosen seed to stderr so a failing run can be
        // replayed verbatim with `--seed <N>`.
        let used_seed = match seed_override {
            Some(n) => {
                seed_rng(n);
                n
            }
            None => seed_rng_from_clock(),
        };
        if seed_override.is_none() {
            eprintln!("seed={}", used_seed);
        }

        // RES-112: short-circuit straight to the token dumper before
        // the rest of the pipeline kicks in. The dumper reads the
        // file, constructs `Lexer::new(src)` (which honours the
        // `logos-lexer` feature flag automatically), drains
        // `next_token_with_span` to EOF, and prints one token per
        // line in `L:C  Kind("lexeme")` form. Exits 0 on success, 1
        // if the file can't be read.
        if dump_tokens {
            if filename.is_empty() {
                eprintln!("Error: --dump-tokens requires a path argument");
                std::process::exit(2);
            }
            match fs::read_to_string(filename) {
                Ok(src) => {
                    dump_tokens_to_stdout(&src);
                    return;
                }
                Err(e) => {
                    eprintln!("Error: could not read {}: {}", filename, e);
                    std::process::exit(1);
                }
            }
        }

        // RES-173: --dump-chunks — read the file, parse + compile
        // to bytecode (peephole included per RES-172), and print a
        // stable-format disassembly. Exits after the dump;
        // mutually exclusive with --lsp / --dump-tokens above.
        if dump_chunks {
            if filename.is_empty() {
                eprintln!("Error: --dump-chunks requires a path argument");
                std::process::exit(2);
            }
            let src = match fs::read_to_string(filename) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error: could not read {}: {}", filename, e);
                    std::process::exit(1);
                }
            };
            // Run the parser — bail cleanly on parse errors rather
            // than attempt to compile a malformed program.
            let (program, errs) = parse(&src);
            if !errs.is_empty() {
                for e in errs {
                    eprintln!("Parser error: {}", e);
                }
                std::process::exit(1);
            }
            // RES-073: resolve `use "..."` before the compiler sees
            // the AST, matching the --vm driver path.
            let mut resolved = program;
            let base_dir = std::path::Path::new(filename)
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf();
            let mut loaded = std::collections::HashSet::new();
            if let Err(e) = imports::expand_uses(&mut resolved, &base_dir, &mut loaded) {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            let prog = match compiler::compile(&resolved) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Error: compile failed: {:?}", e);
                    std::process::exit(1);
                }
            };
            let mut buf = String::new();
            disasm::disassemble(&prog, &mut buf).expect("String write is infallible");
            print!("{}", buf);
            return;
        }

        if !filename.is_empty() {
            // Execute a file. RES-027: a failed run exits non-zero so
            // `run_examples.sh` / CI / ops tooling can distinguish
            // success from failure without parsing stdout.
            let run_result = execute_file(
                filename,
                type_check,
                audit,
                emit_cert_dir.as_deref(),
                use_vm,
                use_jit,
                verifier_timeout_ms,
            );
            // RES-174: print cache stats on exit whenever the
            // flag is set, regardless of whether the run
            // succeeded. Stats only reflect JIT usage; `--vm` /
            // tree-walker runs leave the counters untouched, so
            // printing zeros in that case is accurate.
            #[cfg(feature = "jit")]
            if jit_cache_stats {
                let (h, m, c) = jit_backend::cache_stats();
                eprintln!(
                    "jit-cache: hits={} misses={} compiles={}",
                    h, m, c
                );
            }
            #[cfg(not(feature = "jit"))]
            if jit_cache_stats {
                eprintln!(
                    "jit-cache: unavailable (built without `--features jit`)"
                );
            }
            match run_result {
                Ok(_) => {
                    println!("Program executed successfully");
                    return;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    // RES-074: LSP mode takes priority over REPL when --lsp is set.
    // Without the `lsp` feature, print a helpful pointer and exit.
    if lsp_mode {
        #[cfg(feature = "lsp")]
        {
            lsp_server::run();
            return;
        }
        #[cfg(not(feature = "lsp"))]
        {
            eprintln!(
                "--lsp requires the `lsp` feature. Rebuild with:\n  cargo build --features lsp"
            );
            std::process::exit(1);
        }
    }

    // Start the enhanced REPL if no file was provided. RES-026:
    // pass through --examples-dir so the `examples` command can list
    // real files instead of the hardcoded snippets.
    let mut enhanced_repl = repl::EnhancedREPL::with_examples_dir(examples_dir);
    if let Err(e) = enhanced_repl.run() {
        eprintln!("REPL error: {}", e);
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lex the entire input into a Vec<Token>, stopping at (and including) Eof.
    fn tokenize(input: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(input.to_string());
        let mut out = Vec::new();
        loop {
            let tok = lexer.next_token();
            let is_eof = matches!(tok, Token::Eof);
            out.push(tok);
            if is_eof {
                break;
            }
        }
        out
    }

    // RES-108: drive the hand-rolled scanner directly, bypassing the
    // `logos-lexer` feature routing in `Lexer::new`. Only used by the
    // parity tests below — downstream code always goes through
    // `Lexer::new`.
    // RES-109: synthetic ~100 KLoC input for the lexer benchmark,
    // built by concatenating each `.rs` example with identifiers
    // suffixed per-copy so the input actually looks like a big
    // program (and doesn't get optimized away by a cache).
    #[cfg(feature = "logos-lexer")]
    fn build_100kloc_input() -> String {
        use std::fs;
        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut base = String::new();
        for entry in fs::read_dir(&examples_dir).expect("read examples/") {
            let entry = entry.expect("readable dir entry");
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            base.push_str(&fs::read_to_string(&path).expect("read example"));
            base.push('\n');
        }
        // Tight loop: 100 KLoC / ~240 lines-per-example-concat ≈
        // 400 copies. We just keep concatenating copies with a
        // fresh per-copy identifier suffix until the line count is
        // comfortably over 100k.
        let mut out = String::with_capacity(base.len() * 500);
        let mut line_count = 0usize;
        let target_lines = 100_000usize;
        let mut copy = 0u32;
        while line_count < target_lines {
            let suffix = format!("__c{}", copy);
            // Cheap rename: the lexer only cares about token
            // shapes, so swap every `fn main` to `fn main__cN` to
            // avoid duplicate-def complaints if anyone ever runs
            // this through the parser. Good enough for a scanner
            // benchmark.
            let renamed = base.replace("main(", &format!("main{}(", suffix));
            line_count += renamed.lines().count();
            out.push_str(&renamed);
            out.push('\n');
            copy += 1;
        }
        out
    }

    /// RES-109: A/B the logos-based lexer against the hand-rolled
    /// scanner on a ~100 KLoC synthetic input. `cargo test
    /// --release --features logos-lexer -- --ignored --nocapture
    /// tests::lex_bench_100kloc` runs it; `--ignored` keeps it off
    /// the default test suite (it's slow-ish and only meaningful
    /// in release mode). Prints p50 / p99 / mean per lexer so
    /// `benchmarks/lex/run.sh` can capture the output.
    #[cfg(feature = "logos-lexer")]
    #[test]
    #[ignore]
    fn lex_bench_100kloc() {
        use std::time::Instant;

        fn time_runs<F: FnMut() -> usize>(mut f: F, warmup: usize, runs: usize) -> (u128, u128, u128, usize) {
            for _ in 0..warmup {
                let _ = f();
            }
            let mut samples: Vec<u128> = Vec::with_capacity(runs);
            let mut last_tokens = 0usize;
            for _ in 0..runs {
                let t0 = Instant::now();
                last_tokens = f();
                samples.push(t0.elapsed().as_micros());
            }
            samples.sort();
            let p50 = samples[samples.len() / 2];
            let p99 = samples[samples.len() * 99 / 100];
            let mean = samples.iter().sum::<u128>() / samples.len() as u128;
            (p50, p99, mean, last_tokens)
        }

        let input = build_100kloc_input();
        let line_count = input.lines().count();
        println!("RES-109: lex-bench input = {} lines, {} bytes", line_count, input.len());

        // Warm up 2, time 10 — the legacy lexer is char-by-char
        // over a large Vec<char>, on the order of ~10 s per pass
        // on 100 KLoC. 100 iterations (the ticket's nominal target)
        // would push the `cargo test --ignored` run past 30 minutes
        // on typical laptops. Ten samples per path is enough for
        // the p50 / p99 / ratio numbers this bench reports, and it
        // keeps the harness runnable inside a single iteration of
        // the executor loop.
        let (legacy_p50, legacy_p99, legacy_mean, legacy_n) =
            time_runs(|| legacy_tokenize_with_spans(&input).len(), 2, 10);
        let (logos_p50, logos_p99, logos_mean, logos_n) =
            time_runs(|| crate::lexer_logos::tokenize(&input).len(), 2, 10);

        assert_eq!(
            legacy_n, logos_n,
            "token counts diverged between lexers: legacy={} logos={}",
            legacy_n, logos_n
        );

        let ratio_p50 = legacy_p50 as f64 / logos_p50.max(1) as f64;
        let ratio_mean = legacy_mean as f64 / logos_mean.max(1) as f64;

        println!(
            "| lexer   | p50 (us) | p99 (us) | mean (us) | tokens |"
        );
        println!(
            "|---------|----------|----------|-----------|--------|"
        );
        println!(
            "| legacy  | {:>8} | {:>8} | {:>9} | {:>6} |",
            legacy_p50, legacy_p99, legacy_mean, legacy_n,
        );
        println!(
            "| logos   | {:>8} | {:>8} | {:>9} | {:>6} |",
            logos_p50, logos_p99, logos_mean, logos_n,
        );
        println!(
            "ratio p50:  legacy / logos = {:.2}×",
            ratio_p50
        );
        println!(
            "ratio mean: legacy / logos = {:.2}×",
            ratio_mean
        );
    }

    #[cfg(feature = "logos-lexer")]
    fn legacy_tokenize_with_spans(input: &str) -> Vec<(Token, span::Span)> {
        let mut lex = Lexer {
            input: input.chars().collect(),
            position: 0,
            read_position: 0,
            ch: '\0',
            line: 1,
            column: 0,
            last_token_line: 1,
            last_token_column: 1,
            last_token_offset: 0,
            logos_tokens: None,
        };
        lex.read_char();
        // RES-113: mirror the shebang-skip that `Lexer::new` applies
        // on the non-logos path. Without this, the parity test
        // diverges on examples that start with `#!`.
        if lex.ch == '#' && lex.peek_char() == '!' {
            while lex.ch != '\n' && lex.ch != '\0' {
                lex.read_char();
            }
            if lex.ch == '\n' {
                lex.read_char();
            }
        }
        let mut out = Vec::new();
        loop {
            let (tok, span) = lex.next_token_with_span();
            let is_eof = matches!(tok, Token::Eof);
            out.push((tok, span));
            if is_eof {
                break;
            }
        }
        out
    }

    /// RES-108 + RES-110: parity harness — every `.rs` example in
    /// `resilient/examples/` must produce identical `(Token, Span)`
    /// streams from the legacy hand-rolled lexer and the logos-based
    /// one. As of RES-110 the Span comparison is now exhaustive:
    /// token kind, token payload, `start` (line, column, offset), and
    /// `end` (line, column, offset) must all match.
    #[cfg(feature = "logos-lexer")]
    #[test]
    fn lexer_parity_on_all_examples() {
        use std::fs;

        let examples_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let entries = fs::read_dir(&examples_dir)
            .expect("examples/ directory is present in the resilient crate");

        let mut checked = 0usize;
        for entry in entries {
            let entry = entry.expect("readable dir entry");
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let src = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));

            let legacy = legacy_tokenize_with_spans(&src);
            let logos = crate::lexer_logos::tokenize(&src);

            assert_eq!(
                legacy.len(),
                logos.len(),
                "{}: token count differs — legacy={}, logos={}",
                path.display(),
                legacy.len(),
                logos.len(),
            );

            for (i, (l, r)) in legacy.iter().zip(logos.iter()).enumerate() {
                assert_eq!(
                    l.0,
                    r.0,
                    "{}: token #{} differs — legacy={:?}, logos={:?}",
                    path.display(),
                    i,
                    l.0,
                    r.0,
                );
                assert_eq!(
                    l.1,
                    r.1,
                    "{}: token #{} ({:?}) span differs — legacy={:?}, logos={:?}",
                    path.display(),
                    i,
                    l.0,
                    l.1,
                    r.1,
                );
            }
            checked += 1;
        }

        assert!(
            checked > 0,
            "no .rs examples found under {}",
            examples_dir.display(),
        );
    }

    // RES-110: unit tests for the line-table / `pos_from_byte` helpers.
    // These are always compiled (not gated on `logos-lexer`) because
    // they exercise always-compiled code on `Lexer` / the free fn.

    #[test]
    fn line_table_empty_source_has_single_bof_entry() {
        let table = span::build_line_table("");
        assert_eq!(table, vec![0]);
    }

    #[test]
    fn line_table_no_newlines_has_single_entry() {
        let table = span::build_line_table("abc");
        assert_eq!(table, vec![0]);
    }

    #[test]
    fn line_table_newlines_record_byte_after_each() {
        let src = "abc\ndef\nghi";
        let table = span::build_line_table(src);
        assert_eq!(table, vec![0, 4, 8]);
    }

    #[test]
    fn line_table_trailing_newline_adds_final_entry_past_last_line() {
        let src = "abc\ndef\n";
        let table = span::build_line_table(src);
        // Three logical lines: "abc", "def", and the empty line after.
        assert_eq!(table, vec![0, 4, 8]);
    }

    #[test]
    fn pos_from_byte_start_of_file() {
        let src = "abc\ndef";
        let table = span::build_line_table(src);
        assert_eq!(span::pos_from_byte(&table, src, 0), span::Pos::new(1, 1, 0));
    }

    #[test]
    fn pos_from_byte_end_of_file_no_trailing_newline() {
        let src = "abc";
        let table = span::build_line_table(src);
        // Last char 'c' is at byte 2; byte 3 is past the last byte
        // (EOF) and should still land on line 1.
        assert_eq!(span::pos_from_byte(&table, src, 3), span::Pos::new(1, 4, 3));
    }

    #[test]
    fn pos_from_byte_end_of_file_with_trailing_newline() {
        let src = "abc\n";
        let table = span::build_line_table(src);
        // Byte 4 is the start of the implicit empty line after `\n`.
        assert_eq!(span::pos_from_byte(&table, src, 4), span::Pos::new(2, 1, 4));
    }

    #[test]
    fn pos_from_byte_respects_utf8_char_boundaries_for_column() {
        // Each Greek letter is 2 bytes in UTF-8, so column should be
        // counted in characters, not bytes.
        let src = "αβγ";
        let table = span::build_line_table(src);
        // Byte 2 sits between α and β — one full char before it.
        let pos = span::pos_from_byte(&table, src, 2);
        assert_eq!(pos.line, 1);
        assert_eq!(pos.column, 2);
        assert_eq!(pos.offset, 1);
        // Byte 4 sits between β and γ — two full chars before it.
        let pos = span::pos_from_byte(&table, src, 4);
        assert_eq!(pos.column, 3);
        assert_eq!(pos.offset, 2);
    }

    #[test]
    fn pos_from_byte_across_line_with_utf8_content() {
        let src = "αβ\nγδ";
        let table = span::build_line_table(src);
        // α β \n γ δ — bytes: 0,2 / 4 / 5,7; byte 7 is between γ and δ.
        let pos = span::pos_from_byte(&table, src, 7);
        assert_eq!(pos.line, 2);
        assert_eq!(pos.column, 2);
        // Offset is the total char count from BOF: α β \n γ = 4 chars
        // (the newline counts as a character of its own).
        assert_eq!(pos.offset, 4);
    }

    // ---------- Lexer ----------

    #[test]
    fn lexer_handles_identifier_adjacent_to_paren() {
        // Regression for RES-001: the old lexer swallowed the character
        // after every identifier, so `fn add_one(` lost the `(`.
        let tokens = tokenize("fn add_one(int x) {}");
        assert_eq!(
            tokens,
            vec![
                Token::Function,
                Token::Identifier("add_one".into()),
                Token::LeftParen,
                Token::Identifier("int".into()),
                Token::Identifier("x".into()),
                Token::RightParen,
                Token::LeftBrace,
                Token::RightBrace,
                Token::Eof,
            ]
        );
    }

    #[test]
    fn lexer_distinguishes_int_and_float() {
        let tokens = tokenize("let x = 42; let y = 3.14;");
        // Grab the literals in order.
        let literals: Vec<_> = tokens
            .into_iter()
            .filter(|t| matches!(t, Token::IntLiteral(_) | Token::FloatLiteral(_)))
            .collect();
        // 3.14 is chosen as a typical-looking float for the lexer test,
        // NOT as the math constant PI. clippy 1.91+ flags it as
        // approx_constant; the lint is irrelevant for tokenizer fixtures.
        #[allow(clippy::approx_constant)]
        let expected = vec![Token::IntLiteral(42), Token::FloatLiteral(3.14)];
        assert_eq!(literals, expected);
    }

    #[test]
    fn lexer_recognizes_keywords_and_operators() {
        let tokens = tokenize("if true { return; } else { assert(x == 1); }");
        assert!(tokens.contains(&Token::If));
        assert!(tokens.contains(&Token::Else));
        assert!(tokens.contains(&Token::Return));
        assert!(tokens.contains(&Token::Assert));
        assert!(tokens.contains(&Token::BoolLiteral(true)));
        assert!(tokens.contains(&Token::Equal));
    }

    #[test]
    fn lexer_parses_string_literals_with_escapes() {
        let tokens = tokenize(r#"let s = "hi\n";"#);
        let has_string = tokens
            .iter()
            .any(|t| matches!(t, Token::StringLiteral(s) if s == "hi\n"));
        assert!(has_string, "expected StringLiteral(\"hi\\n\") in {:?}", tokens);
    }

    // ---------- Parser ----------

    fn parse(input: &str) -> (Node, Vec<String>) {
        let lexer = Lexer::new(input.to_string());
        let mut parser = Parser::new(lexer);
        let program = parser.parse_program();
        (program, parser.errors)
    }

    #[test]
    fn parser_let_statement_produces_expected_shape() {
        let (program, errors) = parse("let x = 42;");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => {
                assert_eq!(stmts.len(), 1);
                match &stmts[0].node {
                    Node::LetStatement { name, value, .. } => {
                        assert_eq!(name, "x");
                        assert!(matches!(**value, Node::IntegerLiteral { value: 42, .. }));
                    }
                    other => panic!("expected LetStatement, got {:?}", other),
                }
            }
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_function_with_no_parameters() {
        // RES-004: `fn main()` must parse. Historically the parser
        // appeared to reject it, but that was the RES-001 lexer bug
        // eating the `(`. The parameter-list parser itself already
        // handled empty `()`; this test locks that in.
        let (program, errors) = parse("fn main() { let x = 1; }");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { name, parameters, .. } => {
                    assert_eq!(name, "main");
                    assert!(parameters.is_empty(), "expected no params, got {:?}", parameters);
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_function_with_parameters_roundtrips() {
        let (program, errors) = parse("fn add(int a, int b) { return a + b; }");
        assert!(errors.is_empty(), "unexpected errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { name, parameters, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(
                        parameters,
                        &vec![
                            ("int".to_string(), "a".to_string()),
                            ("int".to_string(), "b".to_string())
                        ]
                    );
                }
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    // ---------- Typechecker ----------

    #[test]
    fn typechecker_accepts_valid_program() {
        let (program, errors) = parse("let x = 42; let y = x + 1;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut tc = typechecker::TypeChecker::new();
        assert!(tc.check_program(&program).is_ok());
    }

    // ---------- Interpreter ----------

    #[test]
    fn interpreter_has_println_registered() {
        // RES-003 contract: every fresh Interpreter has `println` callable.
        let interp = Interpreter::new();
        match interp.env.get("println") {
            Some(Value::Builtin { name, .. }) => assert_eq!(name, "println"),
            other => panic!("expected Builtin(println), got {:?}", other),
        }
    }

    #[test]
    fn builtin_println_rejects_too_many_args() {
        let err = builtin_println(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(err.contains("expects 0 or 1"), "err was: {}", err);
    }

    // --- RES-163: `default` as `_` alias in match arms ---

    #[test]
    fn default_arm_exhausts_previously_non_exhaustive_match() {
        // Without a wildcard `_` / `default`, matching on an int
        // scrutinee is non-exhaustive. With `default =>`, the
        // same match passes typecheck and runs correctly at
        // runtime.
        let src = "\
            fn classify(int n) -> string {\n\
                return match n {\n\
                    0 => \"zero\",\n\
                    1 => \"one\",\n\
                    default => \"other\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                return len(classify(42));\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        // Typecheck must pass — `default` counts as a default arm.
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).expect("should typecheck");
        // And execution picks `default` for 42.
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 5), // "other".len()
            other => panic!("expected Int(5), got {:?}", other),
        }
    }

    #[test]
    fn default_and_underscore_are_interchangeable_at_match_position() {
        // Both forms produce the same AST (`Pattern::Wildcard`),
        // so swapping one for the other in an otherwise-identical
        // program yields identical runtime behaviour.
        let src_under = "fn main(int _d) { return match 7 { _ => 1, }; } main(0);";
        let src_default = "fn main(int _d) { return match 7 { default => 1, }; } main(0);";
        for src in [src_under, src_default] {
            let (program, errs) = parse(src);
            assert!(errs.is_empty(), "parse errors in `{}`: {:?}", src, errs);
            let mut interp = Interpreter::new();
            match interp.eval(&program).unwrap() {
                Value::Int(n) => assert_eq!(n, 1, "src: {}", src),
                other => panic!("src {}: expected Int(1), got {:?}", src, other),
            }
        }
    }

    #[test]
    fn default_as_let_binding_name_is_a_parse_error() {
        // Ticket: "`default` as an identifier now becomes a lex
        // error". The lexer emits `Token::Default` instead of
        // `Token::Identifier("default")`, so the parser's
        // let-parser complains about seeing a non-identifier after
        // `let`.
        let src = "fn main(int _d) { let default = 3; return default; } main(0);";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("found `default`")),
            "expected `found `default`` in errors, got: {:?}",
            errs
        );
    }

    #[test]
    fn default_in_arbitrary_expression_position_is_a_parse_error() {
        // Using `default` where any expression is expected must
        // fail — not just after `let`. A `return default;` hits
        // the parser's "expected expression" path since
        // `Token::Default` isn't a prefix operator / atom.
        let src = "fn f() { return default; } f();";
        let (_program, errs) = parse(src);
        assert!(!errs.is_empty(), "expected parse errors for `default` as expr, got none");
    }

    #[test]
    fn default_works_inside_or_pattern_and_with_guards() {
        // Sanity: `default` isn't special — it's a pattern atom
        // just like `_`, so it combines with the other pattern
        // features (guards from RES-159, or-patterns from RES-160).
        let src = "\
            fn main(int _d) {\n\
                let n = 42;\n\
                return match n {\n\
                    0 | 1 => 1,\n\
                    default if n < 100 => 2,\n\
                    default => 3,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 2), // 42 < 100
            other => panic!("expected Int(2), got {:?}", other),
        }
    }

    // --- RES-162: string-literal match patterns ---

    #[test]
    fn string_literal_pattern_matches_exact_string() {
        // `"start"` as a pattern matches only a scrutinee equal
        // to "start". First-match-wins — falls through to `_` on
        // miss.
        let src = "\
            fn dispatch(string cmd) -> string {\n\
                return match cmd {\n\
                    \"start\" => \"starting\",\n\
                    \"stop\"  => \"stopping\",\n\
                    _         => \"unknown\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                let a = dispatch(\"start\");\n\
                let b = dispatch(\"stop\");\n\
                let c = dispatch(\"foo\");\n\
                return len(a) + len(b) + len(c);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // "starting"(8) + "stopping"(8) + "unknown"(7) = 23
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 23),
            other => panic!("expected Int(23), got {:?}", other),
        }
    }

    #[test]
    fn string_literal_pattern_falls_through_to_wildcard() {
        let src = "\
            fn main(int _d) {\n\
                let s = \"nope\";\n\
                return match s {\n\
                    \"yes\" => 1,\n\
                    \"no\"  => 0,\n\
                    _       => 2,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 2),
            other => panic!("expected Int(2), got {:?}", other),
        }
    }

    #[test]
    fn string_literal_pattern_decodes_escapes() {
        // `"a\n"` in a pattern decodes through the same lexer
        // path as string expressions — the pattern matches a
        // runtime string that contains `a` followed by LF, not
        // the literal four characters `a`, `\`, `n`.
        let src = "\
            fn main(int _d) {\n\
                let s = \"a\\n\";\n\
                let t = match s {\n\
                    \"a\\n\" => 1,\n\
                    \"a\\t\" => 2,\n\
                    _        => 0,\n\
                };\n\
                // And a tab input falls into its own arm.\n\
                let u = match \"a\\t\" {\n\
                    \"a\\n\" => 1,\n\
                    \"a\\t\" => 2,\n\
                    _        => 0,\n\
                };\n\
                return t * 10 + u;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 12), // 1*10 + 2
            other => panic!("expected Int(12), got {:?}", other),
        }
    }

    #[test]
    fn string_match_without_wildcard_is_non_exhaustive() {
        // Ticket AC: over the implicit infinite space of String,
        // literal-only arms never cover — same posture as Int.
        let src = "\
            fn f(string s) -> int {\n\
                return match s {\n\
                    \"a\" => 1,\n\
                    \"b\" => 2,\n\
                };\n\
            }\n\
            fn main(int _d) { return f(\"c\"); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("Non-exhaustive match on string"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn string_literal_pattern_empty_string_matches() {
        // Regression: the empty string `""` is a valid pattern
        // and matches only an empty scrutinee. Hand-rolled and
        // logos lexers both produce `Token::StringLiteral("")`.
        let src = "\
            fn describe(string s) -> string {\n\
                return match s {\n\
                    \"\" => \"empty\",\n\
                    _  => \"non-empty\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                return len(describe(\"\")) + len(describe(\"x\"));\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // "empty"(5) + "non-empty"(9) = 14
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 14),
            other => panic!("expected Int(14), got {:?}", other),
        }
    }

    // --- RES-160: or-patterns in match arms ---

    #[test]
    fn or_pattern_int_any_branch_matches() {
        // Weekend classifier — `0 | 6` means Sunday or Saturday.
        let src = "\
            fn day_of_week(int d) -> string {\n\
                return match d {\n\
                    0 | 6 => \"weekend\",\n\
                    1 | 2 | 3 | 4 | 5 => \"weekday\",\n\
                    _ => \"invalid\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                let a = day_of_week(0);\n\
                let b = day_of_week(3);\n\
                let c = day_of_week(6);\n\
                let d = day_of_week(99);\n\
                return len(a) + len(b) + len(c) + len(d);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // weekend(7) + weekday(7) + weekend(7) + invalid(7) = 28
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 28),
            other => panic!("expected Int(28), got {:?}", other),
        }
    }

    #[test]
    fn or_pattern_string_any_branch_matches() {
        let src = "\
            fn classify(string s) -> string {\n\
                return match s {\n\
                    \"yes\" | \"y\" | \"Y\" => \"affirmative\",\n\
                    \"no\"  | \"n\" | \"N\" => \"negative\",\n\
                    _ => \"unknown\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                let a = classify(\"yes\");\n\
                let b = classify(\"N\");\n\
                let c = classify(\"maybe\");\n\
                return len(a) + len(b) + len(c);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // affirmative(11) + negative(8) + unknown(7) = 26
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 26),
            other => panic!("expected Int(26), got {:?}", other),
        }
    }

    #[test]
    fn or_pattern_mismatched_bindings_error() {
        // One branch binds `x`, the other doesn't — rejected at
        // typecheck with the ticket's diagnostic shape.
        let src = "\
            fn f(int n) -> int {\n\
                return match n {\n\
                    x | 0 => 1,\n\
                    _ => 2,\n\
                };\n\
            }\n\
            fn main(int _d) { return f(0); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("or-pattern branches bind different names"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn or_pattern_bool_both_branches_is_exhaustive() {
        // `true | false => ...` should cover the full bool domain.
        let src = "\
            fn main(int _d) {\n\
                let b = true;\n\
                return match b {\n\
                    true | false => 1,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).expect("should typecheck");
    }

    #[test]
    fn or_pattern_wildcard_branch_counts_as_default() {
        // `0 | _ => ...` counts as a default — no further wildcard
        // arm needed even for a non-finite scrutinee (int).
        let src = "\
            fn main(int _d) {\n\
                let n = 7;\n\
                return match n {\n\
                    0 | _ => 1,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).expect("should typecheck");
    }

    #[test]
    fn or_pattern_all_branches_bind_same_name_is_valid() {
        // Matching Rust's shape: identical binding name across
        // branches is accepted. The body can reference the
        // binding unconditionally.
        let src = "\
            fn f(int n) -> int {\n\
                return match n {\n\
                    x | x => x + 1,\n\
                };\n\
            }\n\
            fn main(int _d) { return f(5); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 6),
            other => panic!("expected Int(6), got {:?}", other),
        }
    }

    #[test]
    fn or_pattern_no_match_falls_through() {
        // None of the or-branches match → next arm fires.
        let src = "\
            fn f(int n) -> int {\n\
                return match n {\n\
                    0 | 1 | 2 => 10,\n\
                    _ => 20,\n\
                };\n\
            }\n\
            fn main(int _d) { return f(5); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 20),
            other => panic!("expected Int(20), got {:?}", other),
        }
    }

    // --- RES-159: match arm guards ---

    #[test]
    fn match_guard_true_body_fires() {
        // Pattern matches + guard evaluates true → that arm's body
        // runs. Here `n == 5` matches the `x` binding, guard
        // `x > 0` is true, so we hit "pos".
        let src = "\
            fn describe(int n) -> string {\n\
                return match n {\n\
                    x if x > 0 => \"pos\",\n\
                    _ => \"other\",\n\
                };\n\
            }\n\
            fn main(int _d) { return describe(5); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::String(s) => assert_eq!(s, "pos"),
            other => panic!("expected String(\"pos\"), got {:?}", other),
        }
    }

    #[test]
    fn match_guard_false_falls_through() {
        // Pattern matches but guard fails → next arm. Here `n == -3`
        // matches `x`, but `x > 0` is false, so control falls to the
        // unguarded catch-all returning "other".
        let src = "\
            fn describe(int n) -> string {\n\
                return match n {\n\
                    x if x > 0 => \"pos\",\n\
                    _ => \"other\",\n\
                };\n\
            }\n\
            fn main(int _d) { return describe(-3); }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::String(s) => assert_eq!(s, "other"),
            other => panic!("expected String(\"other\"), got {:?}", other),
        }
    }

    #[test]
    fn match_guard_has_access_to_pattern_binding() {
        // The guard must see the pattern's identifier binding. This
        // is the main ergonomic win of guards — returning a
        // function of the captured value.
        let src = "\
            fn classify(int n) -> string {\n\
                return match n {\n\
                    x if x < 0 => \"negative\",\n\
                    0 => \"zero\",\n\
                    x if x > 100 => \"big\",\n\
                    _ => \"small-positive\",\n\
                };\n\
            }\n\
            fn main(int _d) {\n\
                let a = classify(-5);\n\
                let b = classify(0);\n\
                let c = classify(42);\n\
                let d = classify(999);\n\
                return len(a) + len(b) + len(c) + len(d);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // lens: negative(8) + zero(4) + small-positive(14) + big(3) = 29
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 29),
            other => panic!("expected Int(29), got {:?}", other),
        }
    }

    #[test]
    fn match_guard_binding_does_not_leak_outside_arm() {
        // The identifier binding `x` is visible only inside the
        // arm's guard + body. Referencing it after the match
        // statement must NOT find the binding.
        let src = "\
            fn main(int _d) {\n\
                let result = match 7 {\n\
                    x if x > 0 => \"big\",\n\
                    _ => \"none\",\n\
                };\n\
                // If `x` leaked we'd see it here — assert instead\n\
                // that the match result itself is correct. The\n\
                // typechecker-level leak check is the other tests.\n\
                return len(result);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 3), // "big" length
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn match_guarded_catchall_does_not_count_as_exhaustive() {
        // A `case _ if <guard> =>` is a GUARDED catch-all — it
        // might not fire, so it can't be the only arm on a
        // non-finite scrutinee. Typechecker must reject.
        let src = "\
            fn main(int _d) {\n\
                let s = \"hello\";\n\
                let r = match s {\n\
                    x if len(x) > 0 => 1,\n\
                };\n\
                return r;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("Non-exhaustive match"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn match_guarded_bool_arms_still_require_both_sides() {
        // Guarded `true` and `false` arms don't cover the bool
        // domain — the typechecker flags the missing unguarded
        // coverage.
        let src = "\
            fn main(int _d) {\n\
                let b = true;\n\
                return match b {\n\
                    true if b == true => 1,\n\
                    false => 0,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("missing `true`"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn match_guarded_then_unguarded_wildcard_is_exhaustive() {
        // The canonical "guard first, then unguarded catch-all"
        // pattern must pass typecheck — this is the shape users
        // will reach for most often.
        let src = "\
            fn main(int _d) {\n\
                let n = 5;\n\
                return match n {\n\
                    x if x > 0 => 1,\n\
                    _ => 0,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).expect("should typecheck");
    }

    #[test]
    fn match_non_boolean_guard_is_typecheck_error() {
        // Guard must evaluate to a boolean. An Int guard should be
        // rejected. (Type::Any is tolerated — that's the usual
        // permissive-inference escape hatch.)
        let src = "\
            fn main(int _d) {\n\
                let n = 5;\n\
                return match n {\n\
                    x if x => 1,\n\
                    _ => 0,\n\
                };\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("guard must be a boolean"),
            "err was: {}",
            err
        );
    }

    // --- RES-156: array comprehensions ---

    /// Evaluate a program returning an Int — helper for assertion
    /// round-tripping through parse + eval.
    fn eval_to_int(src: &str) -> i64 {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => n,
            other => panic!("expected Int, got {:?}", other),
        }
    }

    /// Evaluate a program returning an Array — unwrap to Vec<i64>.
    fn eval_to_int_array(src: &str) -> Vec<i64> {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Array(items) => items
                .into_iter()
                .map(|v| match v {
                    Value::Int(n) => n,
                    other => panic!("expected Int items, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn comprehension_simple_map() {
        let src = "\
            fn main(int _d) {\n\
                let xs = [1, 2, 3];\n\
                return [x * 2 for x in xs];\n\
            }\n\
            main(0);\n\
        ";
        assert_eq!(eval_to_int_array(src), vec![2, 4, 6]);
    }

    #[test]
    fn comprehension_map_and_filter() {
        let src = "\
            fn main(int _d) {\n\
                let xs = [1, 2, 3, 4, 5];\n\
                return [x * x for x in xs if x % 2 == 0];\n\
            }\n\
            main(0);\n\
        ";
        assert_eq!(eval_to_int_array(src), vec![4, 16]);
    }

    #[test]
    fn comprehension_binding_does_not_leak() {
        // The `y` binding inside the comprehension must not leak
        // into the enclosing scope. If the desugar used the
        // enclosing scope directly, the outer assertion below
        // would see `y` bound.
        let src = "\
            fn main(int _d) {\n\
                let y = 100;\n\
                let xs = [1, 2, 3];\n\
                let out = [y for y in xs];\n\
                // Outer `y` still 100 — comprehension's `y` was
                // scoped to the IIFE body.\n\
                return y;\n\
            }\n\
            main(0);\n\
        ";
        assert_eq!(eval_to_int(src), 100);
    }

    #[test]
    fn comprehension_accumulator_name_does_not_shadow_user_r() {
        // A user variable literally named `_r` must still be
        // visible inside the comprehension's result expression —
        // the desugar uses `_r$N` (with `$`) which cannot collide
        // with any legal user identifier.
        let src = "\
            fn main(int _d) {\n\
                let _r = 10;\n\
                let xs = [1, 2, 3];\n\
                let out = [x + _r for x in xs];\n\
                // Sum of [11, 12, 13] == 36.\n\
                let s = 0;\n\
                for v in out { s = s + v; }\n\
                return s;\n\
            }\n\
            main(0);\n\
        ";
        assert_eq!(eval_to_int(src), 36);
    }

    #[test]
    fn comprehension_over_set_via_set_items() {
        // Comprehensions iterate over arrays; users lift Sets via
        // `set_items` (RES-149), which returns a sorted Array.
        let src = "\
            fn main(int _d) {\n\
                let s = #{3, 1, 2};\n\
                return [x * 10 for x in set_items(s)];\n\
            }\n\
            main(0);\n\
        ";
        assert_eq!(eval_to_int_array(src), vec![10, 20, 30]);
    }

    #[test]
    fn comprehension_empty_iterable_produces_empty_array() {
        let src = "\
            fn main(int _d) {\n\
                let xs = [];\n\
                return [x for x in xs];\n\
            }\n\
            main(0);\n\
        ";
        assert!(eval_to_int_array(src).is_empty());
    }

    #[test]
    fn comprehension_counter_bumps_for_each_comprehension() {
        // Two comprehensions in the same program must mint
        // distinct accumulator names so nested / sequential uses
        // don't clash. End-to-end correctness of both invocations
        // is sufficient evidence; if the counter didn't bump, the
        // second call would overwrite the first's internal state.
        let src = "\
            fn main(int _d) {\n\
                let xs = [1, 2];\n\
                let a = [x for x in xs];\n\
                let b = [x * 10 for x in xs];\n\
                let sum = 0;\n\
                for v in a { sum = sum + v; }\n\
                for v in b { sum = sum + v; }\n\
                return sum;\n\
            }\n\
            main(0);\n\
        ";
        // [1,2] sum 3 plus [10,20] sum 30 = 33.
        assert_eq!(eval_to_int(src), 33);
    }

    // --- RES-155: struct destructuring let ---

    /// Grab the value bound to `name` from the interpreter's env
    /// after evaluating `src`, or panic with context. Used by the
    /// destructure tests to verify each local got the right
    /// field value.
    fn eval_and_lookup(src: &str, name: &str) -> Value {
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).expect("eval");
        interp
            .env
            .get(name)
            .unwrap_or_else(|| panic!("binding `{}` not found after eval", name))
    }

    #[test]
    fn let_destructure_full_binds_every_field_shorthand() {
        // `let Point { x, y } = p;` binds `x` and `y` locally.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let p = new Point { x: 3, y: 4 };\n\
                let Point { x, y } = p;\n\
                return x + y;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // The destructure lives inside `main`, whose locals
        // disappear on return. Assert on the return value of
        // `main(0)` instead, which is `x + y == 7`.
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 7),
            other => panic!("expected Int(7), got {:?}", other),
        }
    }

    #[test]
    fn let_destructure_renames_field_to_local() {
        // `let Point { x: a, y: b } = p;` binds new locals `a`, `b`.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let p = new Point { x: 5, y: 6 };\n\
                let Point { x: a, y: b } = p;\n\
                return a * 10 + b;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 56),
            other => panic!("expected Int(56), got {:?}", other),
        }
    }

    #[test]
    fn let_destructure_rest_pattern_ignores_remaining_fields() {
        // `let Foo { a, .. } = f;` only binds `a`; b / c silently
        // dropped. Without `..`, the typechecker would reject.
        let src = "\
            struct Foo { int a, int b, int c }\n\
            fn main(int _d) {\n\
                let f = new Foo { a: 1, b: 2, c: 3 };\n\
                let Foo { a, .. } = f;\n\
                return a;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 1),
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn let_destructure_mixed_shorthand_and_rename() {
        // One shorthand field, one renamed — both in the same
        // pattern. `..` ignores the remaining field.
        let src = "\
            struct Foo { int a, int b, int c }\n\
            fn main(int _d) {\n\
                let f = new Foo { a: 1, b: 2, c: 3 };\n\
                let Foo { a, b: mine, .. } = f;\n\
                return a + mine;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn let_destructure_non_exhaustive_without_rest_is_typecheck_error() {
        // Ticket acceptance: missing fields without `..` must
        // produce a typecheck error listing the missing names.
        let src = "\
            struct Foo { int a, int b, int c }\n\
            fn main(int _d) {\n\
                let f = new Foo { a: 1, b: 2, c: 3 };\n\
                let Foo { a } = f;\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("Non-exhaustive destructure of Foo"),
            "err was: {}",
            err
        );
        assert!(
            err.contains("missing field(s) b, c"),
            "expected `b, c` missing-list, got: {}",
            err
        );
    }

    #[test]
    fn let_destructure_unknown_field_is_typecheck_error() {
        // Pattern field doesn't exist on the struct → clean error.
        let src = "\
            struct Foo { int a, int b }\n\
            fn main(int _d) {\n\
                let f = new Foo { a: 1, b: 2 };\n\
                let Foo { zzz } = f;\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).unwrap_err();
        assert!(
            err.contains("Struct Foo has no field `zzz`"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn let_destructure_wrong_struct_name_is_runtime_error() {
        // Pattern struct name must match the value's struct name.
        // The typechecker tolerates this (it walks the value expr
        // only loosely); runtime catches it.
        let src = "\
            struct Foo { int a }\n\
            struct Bar { int a }\n\
            fn main(int _d) {\n\
                let f = new Foo { a: 1 };\n\
                let Bar { a } = f;\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("Destructure expected struct Bar"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn let_destructure_non_struct_value_is_runtime_error() {
        // Destructuring a non-struct value (e.g. an Int) must fail
        // with a clean message.
        let src = "\
            struct Foo { int a }\n\
            fn main(int _d) {\n\
                let x = 42;\n\
                let Foo { a } = x;\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("Cannot destructure non-struct"),
            "err was: {}",
            err
        );
    }

    // keep the helper used by other RES-155 tests — silence the
    // unused warning if this file grows more tests later.
    #[allow(dead_code)]
    fn _exercise_eval_and_lookup() {
        let _ = eval_and_lookup("fn main(int _d) { return 0; } main(0);", "main");
    }

    // --- RES-154: struct-literal field shorthand ---

    /// Extract the (name, value) pairs of the first `StructLiteral`
    /// reached by a depth-first walk. Returns `None` if the program
    /// has no struct literals or if walk hits an unsupported variant
    /// before reaching one.
    fn first_struct_literal_fields(program: &Node) -> Option<Vec<(String, Node)>> {
        fn walk(n: &Node) -> Option<Vec<(String, Node)>> {
            match n {
                Node::StructLiteral { fields, .. } => Some(fields.clone()),
                Node::Program(stmts) => stmts.iter().find_map(|s| walk(&s.node)),
                Node::Function { body, .. } => walk(body),
                Node::Block { stmts, .. } => stmts.iter().find_map(walk),
                Node::LetStatement { value, .. } => walk(value),
                Node::IfStatement { consequence, alternative, .. } => {
                    walk(consequence).or_else(|| alternative.as_ref().and_then(|a| walk(a)))
                }
                _ => None,
            }
        }
        walk(program)
    }

    #[test]
    fn struct_literal_shorthand_desugars_to_field_name_identifier() {
        // `Point { x, y }` expands to `Point { x: x, y: y }` — the
        // AST stores two `(name, Node::Identifier { name })` pairs.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let x = 1; let y = 2;\n\
                let p = new Point { x, y };\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let fields = first_struct_literal_fields(&program).expect("struct literal");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "x");
        assert!(
            matches!(&fields[0].1, Node::Identifier { name, .. } if name == "x"),
            "expected Identifier(x) for x shorthand, got {:?}",
            fields[0].1
        );
        assert_eq!(fields[1].0, "y");
        assert!(
            matches!(&fields[1].1, Node::Identifier { name, .. } if name == "y"),
            "expected Identifier(y) for y shorthand, got {:?}",
            fields[1].1
        );
    }

    #[test]
    fn struct_literal_shorthand_mixed_with_explicit_field() {
        // `Point { x, y: z }` — shorthand first, explicit second.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let x = 1; let z = 9;\n\
                let p = new Point { x, y: z };\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let fields = first_struct_literal_fields(&program).expect("struct literal");
        assert_eq!(fields.len(), 2);
        assert!(
            matches!(&fields[0].1, Node::Identifier { name, .. } if name == "x"),
            "first field should be shorthand"
        );
        // Second field is explicit `y: z` — value is Identifier(z),
        // distinct from the field name `y`.
        assert_eq!(fields[1].0, "y");
        assert!(
            matches!(&fields[1].1, Node::Identifier { name, .. } if name == "z"),
            "second field should be explicit `y: z`, got {:?}",
            fields[1].1
        );
    }

    #[test]
    fn struct_literal_shorthand_explicit_then_shorthand() {
        // Order flipped: explicit first, shorthand second — ensures
        // the parser's comma-handling works in both cases.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let x = 1; let y = 2;\n\
                let p = new Point { x: 7, y };\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let fields = first_struct_literal_fields(&program).expect("struct literal");
        assert_eq!(fields.len(), 2);
        // First is explicit int literal
        assert!(
            matches!(&fields[0].1, Node::IntegerLiteral { value: 7, .. }),
            "first field should be IntegerLiteral(7), got {:?}",
            fields[0].1
        );
        // Second is shorthand Identifier(y)
        assert!(
            matches!(&fields[1].1, Node::Identifier { name, .. } if name == "y")
        );
    }

    #[test]
    fn struct_literal_shorthand_with_trailing_comma() {
        // Trailing comma after a shorthand must be accepted — same
        // as the explicit-form's trailing-comma policy.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let x = 1; let y = 2;\n\
                let p = new Point { x, y, };\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (_program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
    }

    #[test]
    fn struct_literal_shorthand_unbound_name_errors_at_runtime() {
        // No `x` / `y` in scope — the desugared form produces an
        // "Identifier not found" at eval, which is what the
        // ticket's acceptance criterion specifies.
        let src = "\
            struct Point { int x, int y }\n\
            fn main(int _d) {\n\
                let p = new Point { x, y };\n\
                return 0;\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("Identifier not found"),
            "expected unbound-identifier diagnostic, got: {}",
            err
        );
    }

    // --- RES-152: Bytes value type + builtins ---

    fn as_bytes(v: Value) -> Vec<u8> {
        match v {
            Value::Bytes(b) => b,
            other => panic!("expected Value::Bytes, got {:?}", other),
        }
    }

    #[test]
    fn bytes_literal_hex_named_and_printable_escapes() {
        // The three escape forms the ticket's test requires:
        //   - hex (`\x00`, `\x7f`) for non-printable bytes
        //   - named (`\n`, `\t`, `\r`, `\0`, `\\`, `\"`)
        //   - raw printable ASCII (`Hello`)
        let src = "fn main(int _d) { return b\"\\x00\\x7fHello\\n\\t\\\\\\\"\"; } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let v = interp.eval(&program).unwrap();
        let got = match v {
            Value::Return(inner) => as_bytes(*inner),
            other => as_bytes(other),
        };
        assert_eq!(
            got,
            b"\x00\x7fHello\n\t\\\"".to_vec(),
            "decoded bytes mismatch"
        );
    }

    #[test]
    fn bytes_literal_treats_unicode_escape_as_literal() {
        // Ticket: "Unicode escapes are disallowed (this is a byte
        // literal, not a string)." We honor that by NOT
        // interpreting `\u` — the sequence passes through as the
        // literal two bytes `\` + `u`, plus whatever follows. A
        // user writing `b"\u{41}"` does NOT get `b"A"`.
        let src = "fn main(int _d) { return b\"\\u{41}\"; } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let v = interp.eval(&program).unwrap();
        let got = match v {
            Value::Return(inner) => as_bytes(*inner),
            other => as_bytes(other),
        };
        // Must not be `b"A"` (which would be the Unicode
        // interpretation); must contain the raw `\u` bytes.
        assert_ne!(got, b"A".to_vec(), "accidentally interpreted \\u");
        assert!(
            got.starts_with(b"\\u"),
            "expected raw `\\u` bytes at start, got {:?}",
            got
        );
    }

    #[test]
    fn bytes_len_counts_bytes() {
        let v = builtin_bytes_len(&[Value::Bytes(vec![1, 2, 3, 4])]).unwrap();
        match v {
            Value::Int(n) => assert_eq!(n, 4),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn bytes_len_rejects_non_bytes() {
        let err = builtin_bytes_len(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected Bytes"), "err was: {}", err);
    }

    #[test]
    fn bytes_slice_returns_new_bytes() {
        let b = Value::Bytes(vec![10, 20, 30, 40, 50]);
        let v = builtin_bytes_slice(&[b, Value::Int(1), Value::Int(4)]).unwrap();
        assert_eq!(as_bytes(v), vec![20, 30, 40]);
    }

    #[test]
    fn bytes_slice_rejects_out_of_range() {
        let b = Value::Bytes(vec![1, 2, 3]);
        let err = builtin_bytes_slice(&[b.clone(), Value::Int(0), Value::Int(10)])
            .unwrap_err();
        assert!(err.contains("out of range"), "err was: {}", err);
        let err =
            builtin_bytes_slice(&[b.clone(), Value::Int(-1), Value::Int(1)])
                .unwrap_err();
        assert!(err.contains("negative index"), "err was: {}", err);
        let err = builtin_bytes_slice(&[b, Value::Int(2), Value::Int(1)])
            .unwrap_err();
        assert!(err.contains("start must be <= end"), "err was: {}", err);
    }

    #[test]
    fn byte_at_in_bounds_returns_int_0_to_255() {
        let b = Value::Bytes(vec![0, 128, 255]);
        for (i, want) in [(0_i64, 0_i64), (1, 128), (2, 255)] {
            let v = builtin_byte_at(&[b.clone(), Value::Int(i)]).unwrap();
            match v {
                Value::Int(n) => {
                    assert_eq!(n, want);
                    assert!((0..=255).contains(&n), "byte out of 0..255: {}", n);
                }
                other => panic!("expected Int, got {:?}", other),
            }
        }
    }

    #[test]
    fn byte_at_out_of_bounds_errors() {
        let b = Value::Bytes(vec![1, 2, 3]);
        let err = builtin_byte_at(&[b.clone(), Value::Int(3)]).unwrap_err();
        assert!(err.contains("out of range"), "err was: {}", err);
        let err = builtin_byte_at(&[b, Value::Int(-1)]).unwrap_err();
        assert!(err.contains("out of range"), "err was: {}", err);
    }

    #[test]
    fn byte_at_rejects_non_bytes_first_arg() {
        let err = builtin_byte_at(&[Value::Int(1), Value::Int(0)]).unwrap_err();
        assert!(
            err.contains("expected (Bytes, Int)"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn bytes_display_roundtrips_through_hex_escapes() {
        // Value::Bytes prints as `b"..."` with the same escape
        // alphabet the lexer recognizes — so the output is itself
        // a parseable literal.
        let v = Value::Bytes(vec![0x00, 0x41, 0x7F, 0xFF, b'\n']);
        let s = format!("{}", v);
        assert_eq!(s, "b\"\\x00A\\x7f\\xff\\n\"");
    }

    // --- RES-151: env() builtin (read-only) ---

    /// Guard that serializes env-touching tests — `std::env::set_var`
    /// / `remove_var` mutate process state, so parallel tests that
    /// set and read overlap can see each other's writes. Independent
    /// of the RNG lock because the two resources are unrelated.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Build an env-var name unique to this test run so two tests
    /// using `env()` in parallel still don't collide even if the
    /// lock above were skipped. Uses pid + a bumping counter.
    fn env_name(tag: &str) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("RES_151_TEST_{}_{}_{}", tag, std::process::id(), n)
    }

    #[test]
    fn env_returns_ok_for_set_variable() {
        let _g = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = env_name("ok");
        // SAFETY: `set_var` is `unsafe` on newer Rust editions
        // because concurrent readers can observe a torn env
        // table on some platforms. The `ENV_TEST_LOCK` mutex
        // serializes every test that touches the process env,
        // and the key is unique per call via `env_name`, so
        // the race window this API guards against doesn't
        // exist here.
        unsafe { std::env::set_var(&key, "hello"); }
        let got = builtin_env(&[Value::String(key.clone())]).unwrap();
        match got {
            Value::Result { ok: true, payload } => match *payload {
                Value::String(s) => assert_eq!(s, "hello"),
                other => panic!("expected String payload, got {:?}", other),
            },
            other => panic!("expected Ok(_), got {:?}", other),
        }
        // SAFETY: same rationale as the set_var above —
        // serialized by ENV_TEST_LOCK, unique key.
        unsafe { std::env::remove_var(&key); }
    }

    #[test]
    fn env_returns_err_not_set_for_missing_variable() {
        let _g = ENV_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let key = env_name("missing");
        // Belt-and-suspenders: ensure the name is absent. We mint
        // unique names but a test runner with inherited env could
        // still have collisions.
        // SAFETY: same rationale as the set_var above — serialized
        // by ENV_TEST_LOCK, unique key.
        unsafe { std::env::remove_var(&key); }
        let got = builtin_env(&[Value::String(key)]).unwrap();
        match got {
            Value::Result { ok: false, payload } => match *payload {
                Value::String(s) => assert_eq!(s, "not set"),
                other => panic!("expected String payload, got {:?}", other),
            },
            other => panic!("expected Err(_), got {:?}", other),
        }
    }

    #[test]
    fn env_rejects_non_string_key() {
        let err = builtin_env(&[Value::Int(5)]).unwrap_err();
        assert!(err.contains("expected String argument"), "err was: {}", err);
    }

    #[test]
    fn env_rejects_wrong_arity() {
        let err = builtin_env(&[]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "err was: {}", err);
        let err = builtin_env(&[
            Value::String("A".into()),
            Value::String("B".into()),
        ])
        .unwrap_err();
        assert!(err.contains("expected 1 argument"), "err was: {}", err);
    }

    // --- RES-150: seedable SplitMix64 random builtins ---

    /// Guard that serializes tests which assert on exact RNG
    /// sequences. `RNG_STATE` is a process-wide atomic, so under
    /// cargo's default parallel test runner a second test could
    /// reset-and-read between this test's own reset and its reads,
    /// producing nondeterministic pairs. Tests that only assert on
    /// bounds (not exact values) don't need this lock.
    static RNG_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Reset the RNG state to `seed` so each test runs against a
    /// fresh, reproducible stream regardless of what other tests
    /// pulled from the shared atomic.
    fn reset_rng(seed: u64) {
        use std::sync::atomic::Ordering;
        RNG_STATE.store(seed, Ordering::Relaxed);
    }

    #[test]
    fn splitmix64_matches_reference_sequence_for_seed_1() {
        let _g = RNG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Canonical SplitMix64 — values generated by a reference
        // Rust port at rustc 1.x. Any divergence here means the
        // mix constants drifted; lock it down.
        reset_rng(1);
        let expected: [u64; 10] = [
            10451216379200822465,
            13757245211066428519,
            17911839290282890590,
            8196980753821780235,
            8195237237126968761,
            14072917602864530048,
            16184226688143867045,
            9648886400068060533,
            5266705631892356520,
            14646652180046636950,
        ];
        for (i, want) in expected.iter().enumerate() {
            let got = splitmix64_next();
            assert_eq!(got, *want, "index {}: got {}, want {}", i, got, want);
        }
    }

    #[test]
    fn random_int_is_deterministic_under_same_seed() {
        let _g = RNG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_rng(42);
        let a: Vec<i64> = (0..10)
            .map(|_| {
                match builtin_random_int(&[Value::Int(0), Value::Int(1_000_000)])
                    .unwrap()
                {
                    Value::Int(n) => n,
                    other => panic!("expected Int, got {:?}", other),
                }
            })
            .collect();
        reset_rng(42);
        let b: Vec<i64> = (0..10)
            .map(|_| {
                match builtin_random_int(&[Value::Int(0), Value::Int(1_000_000)])
                    .unwrap()
                {
                    Value::Int(n) => n,
                    other => panic!("expected Int, got {:?}", other),
                }
            })
            .collect();
        assert_eq!(a, b, "same seed must produce same sequence");
    }

    #[test]
    fn random_int_stays_in_half_open_range() {
        reset_rng(7);
        for _ in 0..200 {
            let v = builtin_random_int(&[Value::Int(10), Value::Int(20)]).unwrap();
            match v {
                Value::Int(n) => {
                    assert!(
                        (10..20).contains(&n),
                        "value {} outside [10, 20)",
                        n
                    );
                }
                other => panic!("expected Int, got {:?}", other),
            }
        }
    }

    #[test]
    fn random_int_rejects_reversed_bounds() {
        let err =
            builtin_random_int(&[Value::Int(5), Value::Int(5)]).unwrap_err();
        assert!(err.contains("hi must be > lo"), "err was: {}", err);
        let err =
            builtin_random_int(&[Value::Int(10), Value::Int(5)]).unwrap_err();
        assert!(err.contains("hi must be > lo"), "err was: {}", err);
    }

    #[test]
    fn random_int_rejects_non_int_args() {
        let err =
            builtin_random_int(&[Value::Float(1.0), Value::Int(5)]).unwrap_err();
        assert!(err.contains("expected (Int, Int)"), "err was: {}", err);
    }

    #[test]
    fn random_int_rejects_wrong_arity() {
        let err = builtin_random_int(&[Value::Int(5)]).unwrap_err();
        assert!(err.contains("expected 2 arguments"), "err was: {}", err);
    }

    #[test]
    fn random_float_in_unit_interval() {
        reset_rng(123);
        for _ in 0..200 {
            let v = builtin_random_float(&[]).unwrap();
            match v {
                Value::Float(f) => {
                    assert!(
                        (0.0..1.0).contains(&f),
                        "value {} outside [0.0, 1.0)",
                        f
                    );
                }
                other => panic!("expected Float, got {:?}", other),
            }
        }
    }

    #[test]
    fn random_float_rejects_arguments() {
        let err = builtin_random_float(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 0 arguments"), "err was: {}", err);
    }

    #[test]
    fn seed_rng_pins_subsequent_calls() {
        let _g = RNG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Seeding twice with the same value and drawing 5 samples
        // both times must produce the same sequence.
        seed_rng(999);
        let a: Vec<u64> = (0..5).map(|_| splitmix64_next()).collect();
        seed_rng(999);
        let b: Vec<u64> = (0..5).map(|_| splitmix64_next()).collect();
        assert_eq!(a, b);
    }

    // --- RES-149: Set<T> native value type ---

    /// Count elements in a `Value::Set` or panic with context —
    /// keeps the RES-149 tests compact without giving `Value` a
    /// `PartialEq` derive it doesn't otherwise need.
    fn as_set_len(v: Value) -> usize {
        match v {
            Value::Set(s) => s.len(),
            other => panic!("expected Value::Set, got {:?}", other),
        }
    }

    /// Extract a `Value::Bool` or panic.
    fn as_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            other => panic!("expected Value::Bool, got {:?}", other),
        }
    }

    #[test]
    fn set_new_is_empty() {
        let s = builtin_set_new(&[]).unwrap();
        assert_eq!(as_set_len(s), 0);
    }

    #[test]
    fn set_new_rejects_arguments() {
        let err = builtin_set_new(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 0 arguments"), "err was: {}", err);
    }

    #[test]
    fn set_insert_adds_and_dedups() {
        let s = builtin_set_new(&[]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(1)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(2)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(1)]).unwrap();
        // Duplicate insert is a no-op — set semantics.
        assert_eq!(as_set_len(s), 2);
    }

    #[test]
    fn set_insert_rejects_non_hashable_element() {
        let s = builtin_set_new(&[]).unwrap();
        let err = builtin_set_insert(&[s, Value::Float(1.5)]).unwrap_err();
        assert!(
            err.contains("Set element must be"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn set_insert_rejects_non_set_first_arg() {
        let err = builtin_set_insert(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(
            err.contains("first argument must be a Set"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn set_has_reports_membership() {
        let s = builtin_set_new(&[]).unwrap();
        let s = builtin_set_insert(&[s, Value::String("x".into())]).unwrap();
        assert!(as_bool(
            builtin_set_has(&[s.clone(), Value::String("x".into())]).unwrap()
        ));
        assert!(!as_bool(
            builtin_set_has(&[s, Value::String("y".into())]).unwrap()
        ));
    }

    #[test]
    fn set_has_rejects_non_set_first_arg() {
        let err = builtin_set_has(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(err.contains("first argument must be a Set"), "err was: {}", err);
    }

    #[test]
    fn set_remove_drops_element_and_ignores_missing() {
        let s = builtin_set_new(&[]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(1)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(2)]).unwrap();
        let s = builtin_set_remove(&[s, Value::Int(1)]).unwrap();
        assert_eq!(as_set_len(s.clone()), 1);
        // Removing an absent element is a silent no-op.
        let s = builtin_set_remove(&[s, Value::Int(99)]).unwrap();
        assert_eq!(as_set_len(s), 1);
    }

    #[test]
    fn set_remove_rejects_wrong_arity() {
        let err = builtin_set_remove(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 2 arguments"), "err was: {}", err);
    }

    #[test]
    fn set_len_counts_entries() {
        let s = builtin_set_new(&[]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(1)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(2)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(3)]).unwrap();
        match builtin_set_len(&[s]).unwrap() {
            Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn set_items_returns_sorted_array() {
        // set_items sorts for determinism — documenting the
        // contract at the builtin level so a future HashSet→BTreeSet
        // swap on no_std doesn't change observable ordering.
        let s = builtin_set_new(&[]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(3)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(1)]).unwrap();
        let s = builtin_set_insert(&[s, Value::Int(2)]).unwrap();
        match builtin_set_items(&[s]).unwrap() {
            Value::Array(items) => {
                let ints: Vec<i64> = items
                    .into_iter()
                    .map(|v| match v {
                        Value::Int(n) => n,
                        _ => panic!("expected Int items"),
                    })
                    .collect();
                assert_eq!(ints, vec![1, 2, 3]);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn set_literal_parses_and_evaluates() {
        // End-to-end through parser + interpreter — the set literal
        // opener `#{` should build a Value::Set with duplicates
        // collapsed.
        let src = "\
            fn main(int _d) {\n\
                let s = #{1, 2, 3, 2, 1};\n\
                return set_len(s);\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn empty_set_literal_parses() {
        let src = "fn main(int _d) { return set_len(#{}); } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        match interp.eval(&program).unwrap() {
            Value::Int(n) => assert_eq!(n, 0),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn set_literal_rejects_float_element_at_runtime() {
        // The parser accepts arbitrary expressions; MapKey::from_value
        // surfaces the Int/String/Bool restriction at eval.
        let src = "fn main(int _d) { let s = #{1.5}; return 0; } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("Set element must be"),
            "err was: {}",
            err
        );
    }

    // --- RES-147: clock_ms() monotonic builtin ---

    /// Extract a `Value::Int` or panic with context.
    fn as_int(v: Value) -> i64 {
        match v {
            Value::Int(i) => i,
            other => panic!("expected Value::Int, got {:?}", other),
        }
    }

    #[test]
    fn clock_ms_advances_after_sleep() {
        // Per the ticket: sleep 10ms, assert difference is ≥ 9ms
        // and ≤ 50ms. The upper bound is generous so a slow CI
        // scheduler doesn't flake; the lower bound is slightly
        // below the 10ms sleep because std::thread::sleep is only
        // a lower bound and some platforms round to sub-ms timer
        // ticks.
        let t0 = as_int(builtin_clock_ms(&[]).unwrap());
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t1 = as_int(builtin_clock_ms(&[]).unwrap());
        let delta = t1 - t0;
        assert!(
            (9..=50).contains(&delta),
            "expected 9 <= delta <= 50, got {} (t0={}, t1={})",
            delta,
            t0,
            t1
        );
    }

    #[test]
    fn clock_ms_never_goes_backwards() {
        // Monotonicity invariant: ten rapid calls must produce a
        // non-decreasing sequence. `Instant` is monotonic on all
        // supported platforms, but the test documents the
        // contract at the builtin level so a future refactor can't
        // silently regress.
        let mut prev = as_int(builtin_clock_ms(&[]).unwrap());
        for _ in 0..10 {
            let cur = as_int(builtin_clock_ms(&[]).unwrap());
            assert!(
                cur >= prev,
                "clock_ms regressed: {} -> {}",
                prev,
                cur
            );
            prev = cur;
        }
    }

    #[test]
    fn clock_ms_rejects_arguments() {
        let err = builtin_clock_ms(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected 0 arguments"), "err was: {}", err);
    }

    // --- RES-146: trig / log / exp builtins ---

    /// Extract a `Value::Float` or panic with context. Parallels
    /// `as_string` defined alongside the RES-144 tests.
    fn as_float(v: Value) -> f64 {
        match v {
            Value::Float(f) => f,
            other => panic!("expected Value::Float, got {:?}", other),
        }
    }

    /// Assert `|got - expected| < 1e-9` — the precision the ticket
    /// asks for ("Unit tests assert values to 1e-9 precision against
    /// known references"). A tighter tolerance would flake on f64
    /// epsilon drift across architectures.
    fn close(got: f64, expected: f64, tag: &str) {
        let diff = (got - expected).abs();
        assert!(
            diff < 1e-9,
            "{}: expected {}, got {}, diff {}",
            tag,
            expected,
            got,
            diff
        );
    }

    #[test]
    fn sin_cos_tan_zero() {
        close(as_float(builtin_sin(&[Value::Float(0.0)]).unwrap()), 0.0, "sin(0)");
        close(as_float(builtin_cos(&[Value::Float(0.0)]).unwrap()), 1.0, "cos(0)");
        close(as_float(builtin_tan(&[Value::Float(0.0)]).unwrap()), 0.0, "tan(0)");
    }

    #[test]
    fn sin_cos_at_half_pi() {
        // sin(π/2) = 1, cos(π/2) ≈ 0 (the f64 rep of π/2 has a
        // tiny residual so cos is ~6e-17; well within 1e-9).
        let pi_2 = std::f64::consts::FRAC_PI_2;
        close(as_float(builtin_sin(&[Value::Float(pi_2)]).unwrap()), 1.0, "sin(π/2)");
        close(as_float(builtin_cos(&[Value::Float(pi_2)]).unwrap()), 0.0, "cos(π/2)");
    }

    #[test]
    fn tan_pi_over_4() {
        // tan(π/4) = 1.
        let pi_4 = std::f64::consts::FRAC_PI_4;
        close(as_float(builtin_tan(&[Value::Float(pi_4)]).unwrap()), 1.0, "tan(π/4)");
    }

    #[test]
    fn ln_of_e_and_one() {
        let e = std::f64::consts::E;
        close(as_float(builtin_ln(&[Value::Float(e)]).unwrap()), 1.0, "ln(e)");
        close(as_float(builtin_ln(&[Value::Float(1.0)]).unwrap()), 0.0, "ln(1)");
    }

    #[test]
    fn ln_rejects_non_positive() {
        let err = builtin_ln(&[Value::Float(0.0)]).unwrap_err();
        assert!(err.contains("argument must be > 0"), "err was: {}", err);
        let err = builtin_ln(&[Value::Float(-3.0)]).unwrap_err();
        assert!(err.contains("argument must be > 0"), "err was: {}", err);
    }

    #[test]
    fn ln_rejects_int_per_res130() {
        let err = builtin_ln(&[Value::Int(10)]).unwrap_err();
        assert!(
            err.contains("expected Float"),
            "err was: {}",
            err
        );
        assert!(
            err.contains("to_float"),
            "diagnostic must hint at `to_float` bridge: {}",
            err
        );
    }

    #[test]
    fn log_base_2_of_8() {
        // Base-first argument order per the ticket's Notes —
        // "log base 2 of 8" is `log(2.0, 8.0)`, not `log(8.0, 2.0)`.
        close(
            as_float(builtin_log(&[Value::Float(2.0), Value::Float(8.0)]).unwrap()),
            3.0,
            "log_2(8)",
        );
    }

    #[test]
    fn log_rejects_base_one() {
        let err =
            builtin_log(&[Value::Float(1.0), Value::Float(10.0)]).unwrap_err();
        assert!(err.contains("base must not be 1"), "err was: {}", err);
    }

    #[test]
    fn log_rejects_non_positive_base_and_value() {
        let err =
            builtin_log(&[Value::Float(-2.0), Value::Float(8.0)]).unwrap_err();
        assert!(err.contains("base must be > 0"), "err was: {}", err);
        let err =
            builtin_log(&[Value::Float(2.0), Value::Float(0.0)]).unwrap_err();
        assert!(err.contains("value must be > 0"), "err was: {}", err);
    }

    #[test]
    fn exp_zero_one_and_ln_roundtrip() {
        close(as_float(builtin_exp(&[Value::Float(0.0)]).unwrap()), 1.0, "exp(0)");
        let e = std::f64::consts::E;
        close(as_float(builtin_exp(&[Value::Float(1.0)]).unwrap()), e, "exp(1)");
        // ln(exp(x)) ≈ x for a mid-range value
        let x = 2.5;
        let round = builtin_ln(&[builtin_exp(&[Value::Float(x)]).unwrap()]).unwrap();
        close(as_float(round), x, "ln(exp(2.5))");
    }

    #[test]
    fn exp_rejects_non_float() {
        let err = builtin_exp(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected Float"), "err was: {}", err);
    }

    #[test]
    fn sin_rejects_non_float() {
        let err = builtin_sin(&[Value::Int(0)]).unwrap_err();
        assert!(err.contains("expected Float"), "err was: {}", err);
    }

    #[test]
    fn trig_log_exp_arity_errors() {
        assert!(builtin_sin(&[]).unwrap_err().contains("expected 1"));
        assert!(builtin_cos(&[Value::Float(0.0), Value::Float(0.0)])
            .unwrap_err()
            .contains("expected 1"));
        assert!(builtin_log(&[Value::Float(2.0)])
            .unwrap_err()
            .contains("expected 2"));
    }

    // --- RES-145: string builtins — replace / to_upper / to_lower / format ---

    /// Same extractor as `as_string` — but local to the RES-145 block
    /// since it's defined inside RES-144's block below. Using an
    /// inline helper keeps the tests readable.
    fn s145(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected Value::String, got {:?}", other),
        }
    }

    #[test]
    fn replace_substitutes_all_occurrences() {
        let v = builtin_replace(&[
            Value::String("foo bar foo baz foo".into()),
            Value::String("foo".into()),
            Value::String("XX".into()),
        ])
        .unwrap();
        assert_eq!(s145(v), "XX bar XX baz XX");
    }

    #[test]
    fn replace_empty_from_errors() {
        // Rust's str::replace on an empty pattern inserts the
        // replacement between every character — almost always a
        // bug. We hard-error instead.
        let err = builtin_replace(&[
            Value::String("abc".into()),
            Value::String("".into()),
            Value::String("Z".into()),
        ])
        .unwrap_err();
        assert!(err.contains("`from` must be non-empty"), "err was: {}", err);
    }

    #[test]
    fn to_upper_is_ascii_only() {
        // ASCII-only semantics: `to_upper` only maps a..=z → A..=Z;
        // non-ASCII code points pass through untouched. Without this
        // choice, Turkish locale conventions would capitalize a
        // dotless `i` to `İ` (U+0130), breaking case-insensitive
        // equality on ASCII-only inputs.
        let v = builtin_to_upper(&[Value::String("ábc xYz".into())]).unwrap();
        assert_eq!(s145(v), "áBC XYZ");
    }

    #[test]
    fn to_upper_rejects_non_string() {
        let err = builtin_to_upper(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected string"), "err was: {}", err);
    }

    #[test]
    fn to_lower_is_ascii_only() {
        let v = builtin_to_lower(&[Value::String("ÁBC XyZ".into())]).unwrap();
        // `Á` untouched (non-ASCII); ASCII letters lowered.
        assert_eq!(s145(v), "Ábc xyz");
    }

    #[test]
    fn to_lower_rejects_non_string() {
        let err = builtin_to_lower(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected string"), "err was: {}", err);
    }

    #[test]
    fn format_interpolates_placeholders_in_order() {
        let v = builtin_format(&[
            Value::String("hello {}, you are {} years old".into()),
            Value::Array(vec![
                Value::String("alice".into()),
                Value::Int(30),
            ]),
        ])
        .unwrap();
        assert_eq!(s145(v), "hello alice, you are 30 years old");
    }

    #[test]
    fn format_escapes_double_braces() {
        // `{{` and `}}` collapse to literal `{` / `}`. The remaining
        // `{}` still consumes an arg.
        let v = builtin_format(&[
            Value::String("{{ literal }} then {}".into()),
            Value::Array(vec![Value::Int(7)]),
        ])
        .unwrap();
        assert_eq!(s145(v), "{ literal } then 7");
    }

    #[test]
    fn format_errors_on_too_few_args() {
        let err = builtin_format(&[
            Value::String("a={} b={}".into()),
            Value::Array(vec![Value::Int(1)]),
        ])
        .unwrap_err();
        assert!(
            err.contains("not enough arguments"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn format_errors_on_too_many_args() {
        let err = builtin_format(&[
            Value::String("a={}".into()),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
        ])
        .unwrap_err();
        assert!(err.contains("too many arguments"), "err was: {}", err);
    }

    #[test]
    fn format_errors_on_unmatched_close_brace() {
        let err = builtin_format(&[
            Value::String("close }here".into()),
            Value::Array(vec![]),
        ])
        .unwrap_err();
        assert!(err.contains("unmatched `}`"), "err was: {}", err);
    }

    #[test]
    fn format_errors_on_unsupported_specifier() {
        // `{:04}` is printf-style, out of scope for this MVP.
        let err = builtin_format(&[
            Value::String("{:04}".into()),
            Value::Array(vec![Value::Int(1)]),
        ])
        .unwrap_err();
        assert!(err.contains("unexpected `{`"), "err was: {}", err);
    }

    #[test]
    fn format_rejects_non_array_second_arg() {
        let err = builtin_format(&[
            Value::String("{}".into()),
            Value::Int(42),
        ])
        .unwrap_err();
        assert!(err.contains("expected (string, array)"), "err was: {}", err);
    }

    // --- RES-144: input() builtin ---

    /// Extract the String payload of a `Value::String` or panic with
    /// context if the variant is wrong. Keeps the input tests concise
    /// even though `Value` lacks `PartialEq`.
    fn as_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            other => panic!("expected Value::String, got {:?}", other),
        }
    }

    #[test]
    fn do_input_reads_single_line_and_strips_newline() {
        // Basic happy path — no prompt, one line of data, trailing
        // newline stripped from the returned String.
        let mut r = std::io::Cursor::new(b"alice\n" as &[u8]);
        let v = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v), "alice");
    }

    #[test]
    fn do_input_strips_crlf_line_endings() {
        // Windows-style line endings: both the `\r` and the `\n` are
        // stripped so downstream `if name == "alice"` comparisons
        // behave the same on either platform.
        let mut r = std::io::Cursor::new(b"alice\r\n" as &[u8]);
        let v = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v), "alice");
    }

    #[test]
    fn do_input_returns_empty_string_on_eof() {
        // EOF before any bytes is NOT an error per the ticket's
        // Notes: users can write `while input("> ") != "quit"` and
        // have ctrl-D exit the loop cleanly.
        let mut r = std::io::Cursor::new(b"" as &[u8]);
        let v = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v), "");
    }

    #[test]
    fn do_input_reads_only_first_line_if_multiple_present() {
        // `read_line` stops at the first `\n`; the rest stays in the
        // reader. A follow-up call would return the next line.
        let mut r = std::io::Cursor::new(b"first\nsecond\n" as &[u8]);
        let v1 = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v1), "first");
        let v2 = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v2), "second");
    }

    #[test]
    fn do_input_line_without_trailing_newline_still_returned() {
        // If stdin ends with data but no trailing newline (e.g. piped
        // heredoc), the bytes are still returned — `read_line`
        // considers this a successful read.
        let mut r = std::io::Cursor::new(b"no-newline" as &[u8]);
        let v = do_input(&mut r, "").unwrap();
        assert_eq!(as_string(v), "no-newline");
    }

    #[test]
    fn builtin_input_rejects_non_string_prompt() {
        let err = builtin_input(&[Value::Int(42)]).unwrap_err();
        assert!(
            err.contains("expected String prompt"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn builtin_input_rejects_wrong_arity() {
        let err = builtin_input(&[]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "err was: {}", err);
        let err = builtin_input(&[
            Value::String("a".into()),
            Value::String("b".into()),
        ])
        .unwrap_err();
        assert!(err.contains("expected 1 argument"), "err was: {}", err);
    }

    // --- RES-143: file_read / file_write builtins ---

    /// Create a fresh path in the OS temp dir for round-trip tests.
    /// Using pid + counter keeps parallel test runs from colliding;
    /// the file is cleaned up at the end of each test.
    fn tmp_path(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "res_143_{}_{}_{}.tmp",
            tag,
            std::process::id(),
            n
        ))
    }

    #[test]
    fn file_write_then_file_read_round_trips() {
        let path = tmp_path("roundtrip");
        let path_str = path.to_string_lossy().to_string();
        let contents = "hello, resilient\nline two\n".to_string();

        builtin_file_write(&[
            Value::String(path_str.clone()),
            Value::String(contents.clone()),
        ])
        .expect("file_write");

        let read_back = builtin_file_read(&[Value::String(path_str.clone())]).expect("file_read");
        match read_back {
            Value::String(s) => assert_eq!(s, contents),
            other => panic!("expected String, got {:?}", other),
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_read_errors_on_missing_file() {
        let path = tmp_path("missing");
        // Ensure it doesn't exist.
        let _ = std::fs::remove_file(&path);
        let err = builtin_file_read(&[Value::String(path.to_string_lossy().to_string())])
            .unwrap_err();
        assert!(
            err.starts_with("file_read:"),
            "expected `file_read:` prefix, got: {}",
            err
        );
    }

    #[test]
    fn file_read_errors_on_non_utf8_contents() {
        let path = tmp_path("nonutf8");
        // 0xFF is never a valid UTF-8 start byte.
        std::fs::write(&path, [0xFFu8, 0xFE, 0xFD]).expect("write bytes");
        let err = builtin_file_read(&[Value::String(path.to_string_lossy().to_string())])
            .unwrap_err();
        assert!(
            err.contains("not valid UTF-8"),
            "expected UTF-8 error, got: {}",
            err
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_read_rejects_wrong_arity() {
        let err = builtin_file_read(&[]).unwrap_err();
        assert!(err.contains("expected 1 argument"), "got: {}", err);
        let err = builtin_file_read(&[Value::Int(1)]).unwrap_err();
        assert!(err.contains("expected String"), "got: {}", err);
    }

    #[test]
    fn file_write_rejects_wrong_arity_and_types() {
        let err = builtin_file_write(&[Value::String("x".into())]).unwrap_err();
        assert!(err.contains("expected 2 arguments"), "got: {}", err);
        let err = builtin_file_write(&[Value::Int(1), Value::String("x".into())]).unwrap_err();
        assert!(err.contains("expected (String, String)"), "got: {}", err);
    }

    // --- RES-148: Map builtins + literal syntax ---

    #[test]
    fn map_new_returns_empty_map() {
        let m = builtin_map_new(&[]).unwrap();
        match m {
            Value::Map(m) => assert_eq!(m.len(), 0),
            other => panic!("expected Map, got {:?}", other),
        }
    }

    #[test]
    fn map_insert_then_get_round_trip() {
        let m = builtin_map_new(&[]).unwrap();
        let m = builtin_map_insert(&[
            m,
            Value::String("a".into()),
            Value::Int(1),
        ])
        .unwrap();
        let r = builtin_map_get(&[m, Value::String("a".into())]).unwrap();
        match r {
            Value::Result { ok: true, payload } => match *payload {
                Value::Int(1) => {}
                other => panic!("expected Int(1), got {:?}", other),
            },
            other => panic!("expected Ok(1), got {:?}", other),
        }
    }

    #[test]
    fn map_get_missing_key_returns_err_not_found() {
        let m = builtin_map_new(&[]).unwrap();
        let r = builtin_map_get(&[m, Value::String("nope".into())]).unwrap();
        match r {
            Value::Result { ok: false, payload } => match *payload {
                Value::String(s) => assert_eq!(s, "not found"),
                other => panic!("expected err payload `not found`, got {:?}", other),
            },
            other => panic!("expected Err, got {:?}", other),
        }
    }

    #[test]
    fn map_remove_drops_key() {
        let m = builtin_map_new(&[]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("a".into()), Value::Int(1)]).unwrap();
        let m = builtin_map_remove(&[m, Value::String("a".into())]).unwrap();
        match builtin_map_len(&[m]).unwrap() {
            Value::Int(0) => {}
            other => panic!("expected Int(0), got {:?}", other),
        }
    }

    #[test]
    fn map_keys_returns_sorted_array() {
        let m = builtin_map_new(&[]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("b".into()), Value::Int(2)]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("a".into()), Value::Int(1)]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("c".into()), Value::Int(3)]).unwrap();
        let ks = builtin_map_keys(&[m]).unwrap();
        match ks {
            Value::Array(items) => {
                let strs: Vec<String> = items
                    .into_iter()
                    .map(|v| match v {
                        Value::String(s) => s,
                        other => panic!("non-string key, got {:?}", other),
                    })
                    .collect();
                assert_eq!(strs, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn map_len_counts_entries() {
        let m = builtin_map_new(&[]).unwrap();
        let m = builtin_map_insert(&[m, Value::Int(1), Value::String("one".into())]).unwrap();
        let m = builtin_map_insert(&[m, Value::Int(2), Value::String("two".into())]).unwrap();
        match builtin_map_len(&[m]).unwrap() {
            Value::Int(2) => {}
            other => panic!("expected Int(2), got {:?}", other),
        }
    }

    #[test]
    fn map_insert_rejects_non_hashable_key() {
        // Float isn't a hashable key — must surface the key-type
        // error (source of which is `MapKey::from_value`).
        let m = builtin_map_new(&[]).unwrap();
        let err = builtin_map_insert(&[m, Value::Float(1.5), Value::Int(1)]).unwrap_err();
        assert!(
            err.contains("Map key must be Int, String, or Bool"),
            "expected key-type error, got: {}",
            err,
        );
    }

    #[test]
    fn map_insert_overwrites_existing_key() {
        let m = builtin_map_new(&[]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("a".into()), Value::Int(1)]).unwrap();
        let m = builtin_map_insert(&[m, Value::String("a".into()), Value::Int(99)]).unwrap();
        let r = builtin_map_get(&[m, Value::String("a".into())]).unwrap();
        match r {
            Value::Result { ok: true, payload } => match *payload {
                Value::Int(99) => {}
                other => panic!("expected Int(99), got {:?}", other),
            },
            other => panic!("expected Ok(99), got {:?}", other),
        }
    }

    #[test]
    fn map_literal_parses_and_evaluates() {
        // End-to-end: the new `{k -> v}` syntax through the parser
        // and the interpreter's `MapLiteral` arm.
        let (p, errs) = parse(r#"let m = {"a" -> 1, "b" -> 2};"#);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("m").unwrap() {
            Value::Map(m) => {
                assert_eq!(m.len(), 2);
                assert!(matches!(m.get(&MapKey::Str("a".into())), Some(Value::Int(1))));
                assert!(matches!(m.get(&MapKey::Str("b".into())), Some(Value::Int(2))));
            }
            other => panic!("expected Map, got {:?}", other),
        }
    }

    #[test]
    fn map_literal_accepts_heterogeneous_hashable_keys() {
        // Int, String, Bool all work in the same map literal.
        let (p, errs) = parse(r#"let m = {1 -> "one", "two" -> 2, true -> 3};"#);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("m").unwrap() {
            Value::Map(m) => assert_eq!(m.len(), 3),
            other => panic!("expected Map, got {:?}", other),
        }
    }

    #[test]
    fn map_literal_empty_braces() {
        let (p, errs) = parse("let m = {};");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("m").unwrap() {
            Value::Map(m) => assert_eq!(m.len(), 0),
            other => panic!("expected empty Map, got {:?}", other),
        }
    }

    // --- RES-153: struct field assignment ---

    #[test]
    fn struct_field_assign_one_deep_mutates_field() {
        let src = "\
            struct Point { int x, int y, }\n\
            let p = new Point { x: 1, y: 2 };\n\
            p.x = 42;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        let p = interp.env.get("p").expect("binding `p`");
        match p {
            Value::Struct { fields, .. } => {
                let x = fields.iter().find(|(n, _)| n == "x").map(|(_, v)| v);
                assert!(matches!(x, Some(Value::Int(42))), "got x = {:?}", x);
            }
            other => panic!("expected Struct, got {:?}", other),
        }
    }

    #[test]
    fn struct_field_assign_two_deep_mutates_nested_field() {
        let src = "\
            struct Point { int x, int y, }\n\
            struct Line { Point a, Point b, }\n\
            let l = new Line { \
                a: new Point { x: 1, y: 2 }, \
                b: new Point { x: 3, y: 4 } \
            };\n\
            l.a.x = 99;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        let l = interp.env.get("l").expect("binding `l`");
        let Value::Struct { fields, .. } = l else {
            panic!("expected Line struct, got {:?}", l);
        };
        let a = fields.iter().find(|(n, _)| n == "a").map(|(_, v)| v);
        let Some(Value::Struct { fields: a_fields, .. }) = a else {
            panic!("expected Point struct for a, got {:?}", a);
        };
        let x = a_fields.iter().find(|(n, _)| n == "x").map(|(_, v)| v);
        assert!(matches!(x, Some(Value::Int(99))), "got x = {:?}", x);
        // b is unchanged.
        let b = fields.iter().find(|(n, _)| n == "b").map(|(_, v)| v);
        let Some(Value::Struct { fields: b_fields, .. }) = b else {
            panic!("expected Point struct for b, got {:?}", b);
        };
        let bx = b_fields.iter().find(|(n, _)| n == "x").map(|(_, v)| v);
        assert!(matches!(bx, Some(Value::Int(3))), "b.x should be unchanged, got {:?}", bx);
    }

    // --- RES-158: impl blocks + method calls ---

    #[test]
    fn impl_block_method_call_dispatches_to_mangled_fn() {
        let src = "\
            struct Point { int x, int y, }\n\
            impl Point {\n\
                fn mag_sq(self) -> int {\n\
                    return self.x * self.x + self.y * self.y;\n\
                }\n\
            }\n\
            let p = new Point { x: 3, y: 4 };\n\
            let m = p.mag_sq();\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        let m = interp.env.get("m").expect("binding `m`");
        assert!(matches!(m, Value::Int(25)), "got m = {:?}", m);
        // Mangled fn is also directly callable via its mangled name.
        assert!(matches!(
            interp.env.get("Point$mag_sq"),
            Some(Value::Function { .. })
        ));
    }

    #[test]
    fn impl_block_method_can_call_another_method_on_same_struct() {
        let src = "\
            struct Counter { int n, }\n\
            impl Counter {\n\
                fn one(self) -> int { return self.n; }\n\
                fn two(self) -> int { return self.one() + self.one(); }\n\
            }\n\
            let c = new Counter { n: 7 };\n\
            let r = c.two();\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        let r = interp.env.get("r").expect("binding `r`");
        assert!(matches!(r, Value::Int(14)), "got r = {:?}", r);
    }

    #[test]
    fn impl_block_duplicate_method_is_error() {
        // Two `impl Point { fn m(...) }` blocks with the same method
        // name must surface the duplicate-def diagnostic the ticket
        // calls out.
        let src = "\
            struct Point { int x, }\n\
            impl Point { fn m(self) -> int { return self.x; } }\n\
            impl Point { fn m(self) -> int { return 0; } }\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("duplicate method"),
            "expected duplicate-method diagnostic, got: {}",
            err
        );
        assert!(
            err.contains("Point::m"),
            "expected `Point::m` in diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn struct_field_assign_to_unknown_field_is_typecheck_error() {
        // RES-153: writing to a field the struct didn't declare now
        // fails at typecheck time (pre-runtime).
        let src = "\
            struct Point { int x, int y, }\n\
            let p = new Point { x: 1, y: 2 };\n\
            p.z = 3;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("typecheck should reject unknown field");
        assert!(
            err.contains("has no field `z`"),
            "expected `has no field `z`` in: {}",
            err
        );
        assert!(
            err.contains("available fields: x, y"),
            "expected fields list in: {}",
            err
        );
    }

    #[test]
    fn map_literal_with_non_hashable_key_is_runtime_error() {
        // Float keys are rejected at interpret time — the parser
        // happily accepts anything for the key slot, so this is a
        // runtime check via `MapKey::from_value`.
        let (p, errs) = parse("let m = {1.5 -> 1};");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Map key must be Int, String, or Bool"),
            "expected key-type error, got: {}",
            err,
        );
    }

    #[test]
    fn string_plus_int_coerces() {
        // RES-008: `"x=" + 42` → `"x=42"`
        let (program, errors) = parse(r#"let s = "x=" + 42;"#);
        assert!(errors.is_empty(), "errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "x=42"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn int_plus_string_coerces() {
        let (program, _errors) = parse(r#"let s = 1 + "x";"#);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "1x"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn string_plus_bool_coerces() {
        let (program, _errors) = parse(r#"let s = "on=" + true;"#);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("s").unwrap() {
            Value::String(s) => assert_eq!(s, "on=true"),
            other => panic!("expected String, got {:?}", other),
        }
    }

    #[test]
    fn int_plus_int_still_arithmetic() {
        // Regression: make sure coercion didn't hijack pure-int `+`.
        let (program, _errors) = parse("let n = 1 + 2;");
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.env.get("n").unwrap() {
            Value::Int(n) => assert_eq!(n, 3),
            other => panic!("expected Int(3), got {:?}", other),
        }
    }

    #[test]
    fn lexer_dot_token_and_float_literal_coexist() {
        // RES-010 used to assert Token::Unknown('.'), but RES-038
        // promotes `.` to a real token for field access. Numeric
        // literals with decimals (1.5) still lex correctly because
        // read_number consumes the `.` before the outer dispatcher
        // gets a chance.
        let tokens = tokenize(". 1.5");
        assert_eq!(tokens[0], Token::Dot);
        assert!(
            tokens.iter().any(|t| matches!(t, Token::FloatLiteral(f) if *f == 1.5)),
            "expected FloatLiteral(1.5) to follow, got {:?}",
            tokens
        );
    }

    // ---------- Array builtins (RES-033) ----------

    #[test]
    fn push_returns_new_array() {
        let (p, _e) = parse("let a = [1, 2]; let b = push(a, 3);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(v[2], Value::Int(3)));
            }
            other => panic!("{:?}", other),
        }
        // original untouched
        match interp.env.get("a").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 2),
            _ => panic!("expected Array"),
        }
    }

    #[test]
    fn pop_returns_shorter_array() {
        let (p, _e) = parse("let a = [1, 2, 3]; let b = pop(a);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 2),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn pop_empty_errors() {
        let (p, _e) = parse("let a = []; let b = pop(a);");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("empty"), "{}", err);
    }

    #[test]
    fn slice_returns_subrange() {
        let (p, _e) = parse("let a = [10, 20, 30, 40]; let b = slice(a, 1, 3);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("b").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 2);
                assert!(matches!(v[0], Value::Int(20)));
                assert!(matches!(v[1], Value::Int(30)));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn slice_out_of_range_errors() {
        let (p, _e) = parse("let a = [1]; let b = slice(a, 0, 5);");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("invalid"), "{}", err);
    }

    // ---------- Verification audit (RES-066) ----------

    #[test]
    fn audit_counts_discharged_and_runtime() {
        let (program, _e) = parse(r#"
            fn pos(int x) requires x > 0 { return x; }
            let r1 = pos(5);
            let n = 7;
            let r2 = pos(n);
            let dyn_val = pos(r1);  // r1's type is Any → not foldable
        "#);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).unwrap();
        assert!(
            tc.stats.requires_discharged_at_compile >= 2,
            "got {} discharged",
            tc.stats.requires_discharged_at_compile
        );
        assert_eq!(tc.stats.contracted_call_sites, 3);
    }

    // ---------- Caller-requires propagation (RES-065) ----------

    #[test]
    fn caller_requires_chains_to_callee() {
        // pos requires x > 0; caller requires n == 5; pos(n) holds
        // because 5 > 0 is statically true.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int n) requires n == 5 {
                let r = pos(n);
            }
        "#).unwrap();
    }

    #[test]
    fn caller_requires_violates_callee_caught() {
        // caller asserts n == 0; calls pos(n); pos requires x > 0;
        // 0 > 0 is false → reject.
        let err = typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int n) requires n == 0 {
                let r = pos(n);
            }
        "#).unwrap_err();
        assert!(err.contains("Contract violation"), "got: {}", err);
    }

    #[test]
    fn caller_without_requires_still_works() {
        // No assumptions to propagate; call still folds normally.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int n) {
                let r = pos(5);
            }
        "#).unwrap();
    }

    #[test]
    fn caller_requires_does_not_leak_across_functions() {
        // The assumption is restored at end of body, so a later fn
        // sees an unconstrained n.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn first(int n) requires n == 5 {
                let a = pos(n);
            }
            fn second(int n) {
                // n is unconstrained here — pos(n) must fall to runtime,
                // not be rejected by stale assumptions.
                let b = pos(n);
            }
        "#).unwrap();
    }

    // ---------- Flow-sensitive if-branch assumptions (RES-064) ----------

    #[test]
    fn if_branch_assumption_satisfies_contract() {
        // We assume `x == 5` inside the consequence; pos requires x > 0;
        // 5 > 0 is true, so discharged.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int x) {
                if x == 5 {
                    let r = pos(x);
                }
            }
        "#).unwrap();
    }

    #[test]
    fn if_branch_assumption_rejects_violating_call() {
        // We assume `x == 0` inside the consequence; pos requires x > 0;
        // 0 > 0 is false → reject.
        let err = typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int x) {
                if x == 0 {
                    let r = pos(x);
                }
            }
        "#).unwrap_err();
        assert!(err.contains("Contract violation"), "got: {}", err);
    }

    #[test]
    fn if_branch_assumption_does_not_leak_outside() {
        // After the if, x's value is unknown again.
        // pos(x) outside the if → not rejected (left for runtime)
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int x) {
                if x == 0 {
                    let r = pos(x + 1);  // would fold x=0 + 1 = 1 > 0 → ok
                }
                let r2 = pos(x);  // x's assumption is gone now
            }
        "#).unwrap();
    }

    #[test]
    fn if_literal_eq_ident_form_works_too() {
        // `5 == x` form, not just `x == 5`.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int x) {
                if 5 == x {
                    let r = pos(x);
                }
            }
        "#).unwrap();
    }

    // ---------- Elide proven runtime checks (RES-068) ----------

    #[test]
    fn elide_runtime_check_when_all_callsites_proven() {
        // Both call sites have constant args that satisfy `requires`.
        // The typechecker discharges them; with_proven_fns then
        // strips the runtime check from the bound Function value.
        let src = r#"
            fn pos(int x) requires x > 0 { return x; }
            let a = pos(5);
            let b = pos(10);
        "#;
        let (program, _e) = parse(src);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).unwrap();
        let proven = tc.stats.fully_provable_fns();
        assert!(proven.contains("pos"), "pos should be fully proven");

        // Run with the proven set: pos's Value::Function has empty
        // requires, so apply_function never enters the runtime-check
        // branch even if we deliberately try to violate the contract.
        // Sanity check by inspecting the bound Function value.
        let mut interp = Interpreter::new().with_proven_fns(proven);
        interp.eval(&program).unwrap();
        match interp.env.get("pos").unwrap() {
            Value::Function { requires, .. } => {
                assert!(requires.is_empty(),
                    "expected requires to be elided after proof");
            }
            other => panic!("expected Function, got {:?}", other),
        }
    }

    #[test]
    fn keep_runtime_check_when_some_callsite_unproven() {
        // One call site uses a free variable → typechecker can't
        // prove it, so runtime check must stay.
        let src = r#"
            fn pos(int x) requires x > 0 { return x; }
            fn caller(int n) {
                let a = pos(n);   // unproven (n is free here)
                let b = pos(5);   // proven
            }
        "#;
        let (program, _e) = parse(src);
        let mut tc = typechecker::TypeChecker::new();
        tc.check_program(&program).unwrap();
        let proven = tc.stats.fully_provable_fns();
        assert!(!proven.contains("pos"),
            "pos has an unproven call site, should NOT be fully proven");

        let mut interp = Interpreter::new().with_proven_fns(proven);
        interp.eval(&program).unwrap();
        match interp.env.get("pos").unwrap() {
            Value::Function { requires, .. } => {
                assert_eq!(requires.len(), 1,
                    "expected requires to be retained for runtime check");
            }
            other => panic!("expected Function, got {:?}", other),
        }
    }

    // ---------- Const let-binding tracking (RES-063) ----------

    #[test]
    fn const_let_satisfies_contract() {
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let n = 5;
            let r = pos(n);
        "#).unwrap();
    }

    #[test]
    fn const_let_violates_contract() {
        let err = typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let bad = 0;
            let r = pos(bad);
        "#).unwrap_err();
        assert!(err.contains("Contract violation"), "got: {}", err);
    }

    #[test]
    fn const_chain_through_arithmetic() {
        // n is 5, then m is 2*5 = 10 (foldable), then call.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let n = 5;
            let m = n * 2;
            let r = pos(m);
        "#).unwrap();
    }

    #[test]
    fn reassignment_kills_const_tracking() {
        // After `n = read()` (non-foldable), n is no longer constant —
        // even if we then assign 0, the verifier conservatively gives
        // up rather than risk being unsound. So pos(n) should NOT be
        // rejected, just left for runtime.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let n = 5;
            n = 7;          // killed; verifier gives up
            let r = pos(n); // not rejected, runtime check
        "#).unwrap();
    }

    #[test]
    fn shadowing_with_non_const_kills_tracking() {
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let n = 5;
            let n = 10 / 2;  // foldable, still a constant
            let r = pos(n);
        "#).unwrap();
    }

    // ---------- Call-site contract fold (RES-061) ----------

    #[test]
    fn callsite_constant_args_satisfy_requires() {
        typecheck_src(r#"
            fn divide(int a, int b) requires b != 0 { return a / b; }
            let r = divide(10, 5);
        "#).unwrap();
    }

    #[test]
    fn callsite_constant_args_violate_requires() {
        let err = typecheck_src(r#"
            fn divide(int a, int b) requires b != 0 { return a / b; }
            let r = divide(10, 0);
        "#).unwrap_err();
        assert!(
            err.contains("Contract violation") && err.contains("divide"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn callsite_with_inequality_constraints() {
        // `requires x > 0`. Caller passes -3 → reject.
        let err = typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let bad = pos(0 - 3);
        "#).unwrap_err();
        assert!(err.contains("Contract violation"), "unexpected: {}", err);
        // Caller passes 5 → accept.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let good = pos(5);
        "#).unwrap();
    }

    #[test]
    fn callsite_free_variable_argument_left_for_runtime() {
        // The argument is a variable, not a literal — folder gives up
        // and the call type-checks fine.
        typecheck_src(r#"
            fn pos(int x) requires x > 0 { return x; }
            let v = 5;
            let r = pos(v);
        "#).unwrap();
    }

    #[test]
    fn callsite_multiple_clauses_one_violated() {
        let err = typecheck_src(r#"
            fn ranged(int n)
                requires n >= 0
                requires n <= 100
            { return n; }
            let bad = ranged(150);
        "#).unwrap_err();
        assert!(err.contains("Contract violation"), "unexpected: {}", err);
    }

    // ---------- Static contract discharge (RES-060) ----------

    #[test]
    fn contract_tautology_passes_typecheck() {
        // `5 != 0` is provably true — the typechecker folds it.
        typecheck_src(r#"
            fn f() requires 5 != 0 { return 1; }
        "#).unwrap();
    }

    #[test]
    fn contract_contradiction_rejected_at_compile_time() {
        let err = typecheck_src(r#"
            fn f() requires 0 != 0 { return 1; }
        "#).unwrap_err();
        assert!(
            err.contains("contract can never hold"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn contract_literal_false_rejected() {
        let err = typecheck_src(r#"
            fn f() requires false { return 1; }
        "#).unwrap_err();
        assert!(
            err.contains("contract can never hold"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn contract_with_free_variable_not_folded() {
        // `x > 0` can't be proven at compile time; typecheck should
        // accept it and leave the check for runtime.
        typecheck_src(r#"
            fn f(int x) requires x > 0 { return x; }
        "#).unwrap();
    }

    #[test]
    fn contract_complex_arithmetic_folds() {
        // 2 + 3 == 5 → tautology.
        typecheck_src(r#"
            fn f() requires 2 + 3 == 5 { return 1; }
        "#).unwrap();
        // 2 + 3 == 4 → contradiction.
        let err = typecheck_src(r#"
            fn g() requires 2 + 3 == 4 { return 1; }
        "#).unwrap_err();
        assert!(err.contains("never hold"), "unexpected: {}", err);
    }

    // ---------- Typechecker rejection (RES-053) ----------

    fn typecheck_src(src: &str) -> Result<(), String> {
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        typechecker::TypeChecker::new().check_program(&program).map(|_| ())
    }

    #[test]
    fn typecheck_rejects_let_annot_mismatch() {
        let err = typecheck_src(r#"let x: int = "hi";"#).unwrap_err();
        assert!(
            err.contains("let x: int") && err.contains("string"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_accepts_matching_let_annot() {
        typecheck_src("let x: int = 42;").unwrap();
        typecheck_src(r#"let s: string = "hi";"#).unwrap();
    }

    #[test]
    fn typecheck_rejects_int_plus_bool() {
        let err = typecheck_src("let x = 1 + true;").unwrap_err();
        assert!(err.contains("Cannot apply"), "unexpected: {}", err);
    }

    #[test]
    fn typecheck_rejects_fn_return_type_mismatch() {
        let err = typecheck_src(r#"fn f() -> int { return "hi"; }"#).unwrap_err();
        assert!(
            err.contains("return type mismatch"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_accepts_well_typed_fn() {
        typecheck_src("fn add(int a, int b) -> int { return a + b; }").unwrap();
    }

    #[test]
    fn typecheck_rejects_calling_a_non_function() {
        // 42() -> error because 42 has type Int, not a function.
        let err = typecheck_src("let x = 42; let y = x(0);").unwrap_err();
        assert!(
            err.contains("Cannot call non-function"),
            "unexpected: {}",
            err
        );
    }

    #[test]
    fn typecheck_rejects_bitwise_on_float() {
        let err = typecheck_src("let x = 1.5 & 2;").unwrap_err();
        assert!(err.contains("Bitwise"), "unexpected: {}", err);
    }

    #[test]
    fn typecheck_accepts_string_plus_int_coercion() {
        // RES-008 compatibility.
        typecheck_src(r#"let s = "n=" + 42;"#).unwrap();
    }

    #[test]
    fn typecheck_rejects_try_on_non_result() {
        let err = typecheck_src("let x = 42?;").unwrap_err();
        assert!(err.contains("? operator"), "unexpected: {}", err);
    }

    // ---------- Typed declarations (RES-052) ----------

    #[test]
    fn typed_let_parses_and_records_annotation() {
        let (p, errors) = parse("let x: int = 42;");
        assert!(errors.is_empty(), "{:?}", errors);
        match p {
            Node::Program(stmts) => match &stmts[0].node {
                Node::LetStatement { name, value, type_annot, .. } => {
                    assert_eq!(name, "x");
                    assert_eq!(type_annot.as_deref(), Some("int"));
                    assert!(matches!(**value, Node::IntegerLiteral { value: 42, .. }));
                }
                other => panic!("expected LetStatement, got {:?}", other),
            },
            _ => panic!("expected Program"),
        }
    }

    #[test]
    fn typed_let_still_executes() {
        let (p, _e) = parse("let x: int = 42; let y = x + 1;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("y").unwrap(), Value::Int(43)));
    }

    #[test]
    fn fn_with_return_type_parses() {
        let src = r#"
            fn add(int a, int b) -> int {
                return a + b;
            }
            let r = add(2, 3);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        // Find the Function node to check its return_type.
        match p {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { name, return_type, .. } => {
                    assert_eq!(name, "add");
                    assert_eq!(return_type.as_deref(), Some("int"));
                }
                other => panic!("expected Function, got {:?}", other),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn return_type_on_anonymous_fn() {
        let (p, errors) = parse("let f = fn(int x) -> int { return x + 1; };");
        assert!(errors.is_empty(), "{:?}", errors);
        // Execute and confirm behavior is unchanged.
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        // Also confirm it behaves callably.
        let (p2, _e) = parse("let f = fn(int x) -> int { return x + 1; }; let r = f(10);");
        let mut interp2 = Interpreter::new();
        interp2.eval(&p2).unwrap();
        assert!(matches!(interp2.env.get("r").unwrap(), Value::Int(11)));
    }

    // ---------- First-class functions (RES-042) ----------

    #[test]
    fn anonymous_fn_called_inline() {
        let (p, errors) = parse("let add = fn(int a, int b) { return a + b; }; let r = add(2, 3);");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(5)));
    }

    #[test]
    fn closure_with_shared_mutation_makes_a_counter() {
        // RES-056 (unblocked by RES-050): the canonical
        // make_counter test. The inner fn captures `n` from the outer
        // env. Because Environment is now Rc<RefCell<>>, the
        // captured env IS the same RefCell that was mutated, and
        // every call to the closure reads/writes the same `n`.
        //
        // Before RES-050 this would have returned 1, 1, 1 because
        // each call cloned the captured env and mutated the clone.
        let src = r#"
            fn make_counter() {
                let n = 0;
                return fn() {
                    n = n + 1;
                    return n;
                };
            }
            let c = make_counter();
            let a = c();
            let b = c();
            let three = c();
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a").unwrap(), Value::Int(1)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(2)));
        assert!(matches!(interp.env.get("three").unwrap(), Value::Int(3)));
    }

    #[test]
    fn two_counters_are_independent() {
        // Each call to make_counter creates a fresh enclosing env;
        // counters made in different calls must NOT share state.
        let src = r#"
            fn make_counter() {
                let n = 0;
                return fn() {
                    n = n + 1;
                    return n;
                };
            }
            let a = make_counter();
            let b = make_counter();
            let a1 = a();
            let a2 = a();
            let b1 = b();
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a1").unwrap(), Value::Int(1)));
        assert!(matches!(interp.env.get("a2").unwrap(), Value::Int(2)));
        // b is fresh, so b() returns 1, not 3.
        assert!(matches!(interp.env.get("b1").unwrap(), Value::Int(1)));
    }

    #[test]
    fn closure_captures_enclosing_variable() {
        let src = r#"
            fn make_adder(int n) {
                return fn(int x) { return x + n; };
            }
            let add5 = make_adder(5);
            let r = add5(10);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(15)));
    }

    #[test]
    fn anonymous_fn_can_have_contracts() {
        // The anonymous-fn form inherits the full fn parser, including
        // requires/ensures.
        let src = r#"
            let safe_div = fn(int a, int b)
                requires b != 0
            {
                return a / b;
            };
            let r = safe_div(20, 5);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(4)));
    }

    // ---------- Result type (RES-040) ----------

    #[test]
    fn result_ok_and_err_construct() {
        let (p, _e) = parse(r#"
            let good = Ok(42);
            let bad = Err("boom");
            let g_ok = is_ok(good);
            let b_ok = is_ok(bad);
            let g = unwrap(good);
            let b = unwrap_err(bad);
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("g_ok").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("b_ok").unwrap(), Value::Bool(false)));
        assert!(matches!(interp.env.get("g").unwrap(), Value::Int(42)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::String(s) if s == "boom"));
    }

    #[test]
    fn unwrap_on_err_errors() {
        let (p, _e) = parse(r#"let x = unwrap(Err("no"));"#);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("unwrap called on Err"), "{}", err);
    }

    // ---------- ? propagation (RES-041) ----------

    #[test]
    fn try_operator_propagates_err() {
        let src = r#"
            fn parse_int() { return Err("not a number"); }
            fn double() {
                let n = parse_int()?;
                return Ok(n + n);
            }
            let r = double();
            let was_err = is_err(r);
            let msg = unwrap_err(r);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("was_err").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("msg").unwrap(), Value::String(s) if s == "not a number"));
    }

    #[test]
    fn try_operator_extracts_ok() {
        let src = r#"
            fn get() { return Ok(7); }
            fn user() {
                let n = get()?;
                return Ok(n * 3);
            }
            let r = unwrap(user());
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(21)));
    }

    #[test]
    fn try_operator_on_non_result_errors() {
        let (p, _e) = parse("let x = 42?;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("? operator expects a Result"), "{}", err);
    }

    // ---------- String builtins (RES-043) ----------

    #[test]
    fn string_builtins_basic() {
        let (p, _e) = parse(r#"
            let parts = split("a,b,c", ",");
            let t = trim("   hi   ");
            let hasH = contains("hello", "ell");
            let up = to_upper("Foo");
            let lo = to_lower("BAR");
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("parts").unwrap() {
            Value::Array(v) => {
                assert_eq!(v.len(), 3);
                assert!(matches!(&v[1], Value::String(s) if s == "b"));
            }
            _ => panic!("expected Array"),
        }
        assert!(matches!(interp.env.get("t").unwrap(), Value::String(s) if s == "hi"));
        assert!(matches!(interp.env.get("hasH").unwrap(), Value::Bool(true)));
        assert!(matches!(interp.env.get("up").unwrap(), Value::String(s) if s == "FOO"));
        assert!(matches!(interp.env.get("lo").unwrap(), Value::String(s) if s == "bar"));
    }

    #[test]
    fn split_empty_sep_per_char() {
        let (p, _e) = parse(r#"let cs = split("abc", "");"#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("cs").unwrap() {
            Value::Array(v) => assert_eq!(v.len(), 3),
            _ => panic!("expected Array"),
        }
    }

    // ---------- Match exhaustiveness (RES-054) ----------

    #[test]
    fn typecheck_rejects_non_exhaustive_bool_match() {
        let err = typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, };
        "#).unwrap_err();
        assert!(err.contains("Non-exhaustive match on bool"), "got: {}", err);
        assert!(err.contains("missing `false`"), "got: {}", err);
    }

    #[test]
    fn typecheck_accepts_exhaustive_bool_match() {
        typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, false => 0, };
        "#).unwrap();
    }

    #[test]
    fn typecheck_accepts_bool_match_with_wildcard() {
        typecheck_src(r#"
            let b = true;
            let r = match b { true => 1, _ => 0, };
        "#).unwrap();
    }

    #[test]
    fn typecheck_rejects_int_match_without_default() {
        let err = typecheck_src(r#"
            let n = 5;
            let r = match n { 0 => "zero", 1 => "one", };
        "#).unwrap_err();
        assert!(err.contains("Non-exhaustive match on int"), "got: {}", err);
    }

    #[test]
    fn typecheck_accepts_int_match_with_identifier_default() {
        typecheck_src(r#"
            let n = 5;
            let r = match n { 0 => "zero", x => "other", };
        "#).unwrap();
    }

    // ---------- match (RES-039) ----------

    #[test]
    fn match_literal_arm() {
        let src = r#"
            let r = match 2 {
                0 => "zero",
                1 => "one",
                2 => "two",
                n => "other",
            };
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("r").unwrap() {
            Value::String(s) => assert_eq!(s, "two"),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn match_identifier_binding() {
        let src = r#"
            let r = match 42 {
                0 => -1,
                n => n * 2,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(84)));
    }

    #[test]
    fn match_wildcard_falls_through() {
        let src = r#"
            let r = match "nope" {
                "yes" => 1,
                _ => 0,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(0)));
    }

    #[test]
    fn match_no_arm_matches_returns_void() {
        let src = r#"
            let r = match 5 {
                0 => 1,
                1 => 2,
            };
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Void));
    }

    #[test]
    fn match_binding_does_not_leak() {
        // Identifier pattern binding must not escape the match.
        let src = r#"
            let m = match 1 { n => n + 1, };
            let outer = 99;
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        // `n` should NOT be visible outside the match arm.
        assert!(interp.env.get("n").is_none());
        assert!(matches!(interp.env.get("m").unwrap(), Value::Int(2)));
    }

    // ---------- Structs (RES-038) ----------

    #[test]
    fn struct_decl_literal_and_access() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 3, y: 4 };
            let dx = p.x;
            let dy = p.y;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("dx").unwrap(), Value::Int(3)));
        assert!(matches!(interp.env.get("dy").unwrap(), Value::Int(4)));
    }

    #[test]
    fn struct_field_assignment() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 0, y: 0 };
            p.x = 7;
            let got = p.x;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("got").unwrap(), Value::Int(7)));
    }

    #[test]
    fn struct_nested_field_assignment() {
        let src = r#"
            struct Inner { int v, }
            struct Outer { int tag, int v, }
            let o = new Outer { tag: 1, v: 0 };
            o.v = 99;
            let got = o.v;
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("got").unwrap(), Value::Int(99)));
    }

    #[test]
    fn struct_unknown_field_errors() {
        let src = r#"
            struct Point { int x, int y, }
            let p = new Point { x: 1, y: 2 };
            let z = p.z;
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("no field 'z'"), "err: {}", err);
    }

    #[test]
    fn struct_empty() {
        let src = r#"
            struct Empty {}
            let e = new Empty {};
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("e").unwrap() {
            Value::Struct { name, fields } => {
                assert_eq!(name, "Empty");
                assert!(fields.is_empty());
            }
            other => panic!("{:?}", other),
        }
    }

    // ---------- Live-block invariants (RES-036) ----------

    #[test]
    fn live_block_with_passing_invariant() {
        let src = r#"
            let fuel = 100;
            live invariant fuel >= 0 {
                fuel = fuel - 10;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("fuel").unwrap(), Value::Int(90)));
    }

    #[test]
    fn live_block_invariant_violation_retries_then_fails() {
        // This body ALWAYS leaves fuel negative. After three retries
        // the block gives up with an invariant-violation error.
        let src = r#"
            let fuel = 5;
            live invariant fuel >= 0 {
                fuel = fuel - 100;
            }
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Invariant violation") && err.contains("fuel >= 0"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn live_block_multiple_invariants() {
        let src = r#"
            let x = 5;
            let y = 10;
            live
                invariant x >= 0
                invariant y > x
            {
                x = x + 1;
                y = y + 1;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(6)));
        assert!(matches!(interp.env.get("y").unwrap(), Value::Int(11)));
    }

    // ---------- for..in (RES-037) ----------

    #[test]
    fn for_in_sums_array() {
        let src = r#"
            let xs = [1, 2, 3, 4, 5];
            let s = 0;
            for x in xs { s = s + x; }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(15)));
    }

    #[test]
    fn for_in_empty_array_is_noop() {
        let (p, _e) = parse("let s = 99; for x in [] { s = 0; }");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(99)));
    }

    #[test]
    fn for_in_nested_arrays() {
        let src = r#"
            let m = [[1, 2], [3, 4]];
            let s = 0;
            for row in m { for v in row { s = s + v; } }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("s").unwrap(), Value::Int(10)));
    }

    #[test]
    fn for_in_non_array_errors() {
        let (p, _e) = parse("for x in 42 { let y = 1; }");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("iterable must be an array"), "{}", err);
    }

    // ---------- Function contracts (RES-035) ----------

    #[test]
    fn contract_requires_valid_passes() {
        let src = r#"
            fn divide(int a, int b)
                requires b != 0
            {
                return a / b;
            }
            let x = divide(10, 2);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(5)));
    }

    #[test]
    fn contract_requires_violation_errors() {
        let src = r#"
            fn divide(int a, int b)
                requires b != 0
            {
                return a / b;
            }
            let x = divide(10, 0);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Contract violation") && err.contains("requires"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn contract_ensures_valid_passes() {
        let src = r#"
            fn double(int n)
                ensures result == n * 2
            {
                return n + n;
            }
            let x = double(7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(14)));
    }

    #[test]
    fn contract_ensures_violation_errors() {
        let src = r#"
            fn broken_double(int n)
                ensures result == n * 2
            {
                return n + 1;
            }
            let x = broken_double(7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Contract violation") && err.contains("ensures"),
            "unexpected error: {}",
            err
        );
        assert!(
            err.contains("result = 8"),
            "expected result value in error, got: {}",
            err
        );
    }

    #[test]
    fn contract_multiple_clauses() {
        let src = r#"
            fn clamped(int n, int lo, int hi)
                requires lo <= hi
                requires n >= lo
                requires n <= hi
                ensures result == n
            {
                return n;
            }
            let x = clamped(5, 0, 10);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(5)));
    }

    #[test]
    fn array_literal_and_index() {
        let (p, errors) = parse("let a = [10, 20, 30]; let b = a[1];");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(20)));
    }

    #[test]
    fn array_index_assignment() {
        let src = "let a = [1, 2, 3]; a[0] = 99;";
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("a").unwrap() {
            Value::Array(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(items[0], Value::Int(99)));
                assert!(matches!(items[1], Value::Int(2)));
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    #[test]
    fn array_out_of_bounds_errors() {
        let (p, _e) = parse("let a = [1]; let b = a[5];");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of bounds"), "{}", err);
    }

    #[test]
    fn array_concat_and_len() {
        let (p, _e) = parse("let a = [1,2] + [3,4,5]; let n = len(a);");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("n").unwrap(), Value::Int(5)));
        match interp.env.get("a").unwrap() {
            Value::Array(items) => assert_eq!(items.len(), 5),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn empty_array_literal() {
        let (p, errors) = parse("let a = []; let n = len(a);");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("n").unwrap(), Value::Int(0)));
    }

    #[test]
    fn nested_array() {
        let (p, _e) = parse("let m = [[1,2],[3,4]]; let x = m[1][0];");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(3)));
    }

    #[test]
    fn bitwise_operators() {
        let (p, _e) = parse(r#"
            let a = 0xF0 & 0x33;
            let b = 0x01 | 0x02;
            let c = 0xFF ^ 0x0F;
            let d = 1 << 4;
            let e = 256 >> 3;
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a").unwrap(), Value::Int(0x30)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(0x03)));
        assert!(matches!(interp.env.get("c").unwrap(), Value::Int(0xF0)));
        assert!(matches!(interp.env.get("d").unwrap(), Value::Int(16)));
        assert!(matches!(interp.env.get("e").unwrap(), Value::Int(32)));
    }

    #[test]
    fn bitwise_shift_out_of_range_errors() {
        let (p, _e) = parse("let x = 1 << 64;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of range"), "{}", err);
    }

    #[test]
    fn assert_shows_both_operands() {
        // RES-028: when an infix comparison assert fails, both sides
        // appear in the error so the user can see the actual values.
        let src = r#"
            let fuel = -5;
            assert(fuel >= 0, "Fuel must be non-negative");
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("Fuel must be non-negative"), "msg lost: {}", err);
        assert!(
            err.contains("-5 >= 0") || err.contains("condition -5 >= 0"),
            "expected both operands in error, got: {}",
            err
        );
    }

    #[test]
    fn hex_and_binary_literals() {
        let (p, _e) = parse("let a = 0xFF; let b = 0b1010; let c = 0xDEAD_BEEF;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("a").unwrap(), Value::Int(255)));
        assert!(matches!(interp.env.get("b").unwrap(), Value::Int(10)));
        assert!(matches!(interp.env.get("c").unwrap(), Value::Int(0xDEADBEEF)));
    }

    #[test]
    fn block_comments_are_stripped() {
        let src = "let /* inline */ x = /* another */ 42; /* trailing */";
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("x").unwrap(), Value::Int(42)));
    }

    #[test]
    fn block_comment_spanning_lines() {
        let src = "let x = 1;\n/* line two\nand three */\nlet y = 2;";
        let (_p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn while_loop_counts_to_ten() {
        let src = r#"
            let i = 0;
            let sum = 0;
            while i < 10 {
                sum = sum + i;
                i = i + 1;
            }
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("sum").unwrap() {
            Value::Int(n) => assert_eq!(n, 45), // 0+1+..+9
            other => panic!("expected Int(45), got {:?}", other),
        }
    }

    #[test]
    fn while_loop_runaway_is_capped() {
        // A tight `while true` should error out rather than hang.
        let (p, _e) = parse("let x = 0; while true { x = x + 1; }");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("runaway"), "{}", err);
    }

    #[test]
    fn string_comparisons() {
        let (p, _e) = parse(r#"
            let a = "apple" < "banana";
            let b = "abc" == "abc";
            let c = "xy" >= "xz";
            let d = len("héllo");
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let g = |n: &str| interp.env.get(n).unwrap();
        assert!(matches!(g("a"), Value::Bool(true)));
        assert!(matches!(g("b"), Value::Bool(true)));
        assert!(matches!(g("c"), Value::Bool(false)));
        // "héllo" is 5 Unicode scalars.
        assert!(matches!(g("d"), Value::Int(5)));
    }

    #[test]
    fn logical_and_or_evaluate() {
        let (p, _e) = parse(r#"
            let a = true && false;
            let b = true || false;
            let c = false || (1 < 2);
            let d = (5 > 0) && (5 < 10);
        "#);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let g = |n: &str| match interp.env.get(n).unwrap() {
            Value::Bool(b) => b,
            other => panic!("expected Bool for {}, got {:?}", n, other),
        };
        assert!(!g("a"));
        assert!(g("b"));
        assert!(g("c"));
        assert!(g("d"));
    }

    #[test]
    fn if_with_and_or_condition() {
        // Integration with parser: complex conditions in `if`.
        let (_p, errors) = parse("fn f() { if true && false { let x = 1; } }");
        assert!(errors.is_empty(), "{:?}", errors);
    }

    #[test]
    fn math_builtins_sqrt_pow_floor_ceil() {
        // RES-055: pow/floor/ceil are type-preserving — int args
        // produce Int results. sqrt is intentionally still Float.
        let src = r#"
            let a = sqrt(16);
            let b = pow(2, 10);
            let c = floor(3.7);
            let d = ceil(3.2);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let get = |n: &str| interp.env.get(n).unwrap();
        // sqrt(int) is still Float — irrational results are the norm.
        assert!(matches!(get("a"), Value::Float(v) if (v - 4.0).abs() < 1e-9));
        // pow(int, int) is now Int (RES-055).
        assert!(matches!(get("b"), Value::Int(1024)));
        // floor/ceil of float still return Float.
        assert!(matches!(get("c"), Value::Float(v) if (v - 3.0).abs() < 1e-9));
        assert!(matches!(get("d"), Value::Float(v) if (v - 4.0).abs() < 1e-9));
    }

    #[test]
    fn math_builtins_abs_min_max() {
        let src = r#"
            let a = abs(-5);
            let b = abs(-3.5);
            let c = min(3, 7);
            let d = max(3.0, 7);
        "#;
        let (p, _e) = parse(src);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let get = |n: &str| interp.env.get(n).unwrap();
        assert!(matches!(get("a"), Value::Int(5)));
        assert!(matches!(get("b"), Value::Float(v) if (v - 3.5).abs() < 1e-9));
        assert!(matches!(get("c"), Value::Int(3)));
        assert!(matches!(get("d"), Value::Float(v) if (v - 7.0).abs() < 1e-9));
    }

    #[test]
    fn math_builtins_arity_checks() {
        let e_abs = builtin_abs(&[Value::Int(1), Value::Int(2)]).unwrap_err();
        assert!(e_abs.contains("expected 1"), "{}", e_abs);
        let e_min = builtin_min(&[Value::Int(1)]).unwrap_err();
        assert!(e_min.contains("expected 2"), "{}", e_min);
    }

    #[test]
    fn recursive_function_with_params() {
        // Regression for the apply_function self-bind fix: a fn with
        // params that recursively calls itself used to fail with
        // "Identifier not found: <fn name>" because the captured env
        // didn't include the function's own re-bound version.
        let src = r#"
            fn fib(int n) {
                if n < 2 { return n; }
                return fib(n - 1) + fib(n - 2);
            }
            let r = fib(10);
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        assert!(matches!(interp.env.get("r").unwrap(), Value::Int(55)));
    }

    #[test]
    fn forward_reference_between_functions() {
        // RES-018: caller is defined before callee, which only works if
        // eval_program hoists function definitions.
        let src = r#"
            fn caller() { return callee(); }
            fn callee() { return 42; }
            let x = caller();
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn static_let_persists_across_calls() {
        // RES-013: counter survives across calls. Three calls → 1, 2, 3.
        let src = r#"
            fn tick() {
                static let n = 0;
                n = n + 1;
                return n;
            }
            let a = tick();
            let b = tick();
            let c = tick();
        "#;
        let (p, errors) = parse(src);
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let extract = |name: &str| match interp.env.get(name).unwrap() {
            Value::Int(n) => n,
            other => panic!("expected Int for {}, got {:?}", name, other),
        };
        assert_eq!(extract("a"), 1);
        assert_eq!(extract("b"), 2);
        assert_eq!(extract("c"), 3);
    }

    #[test]
    fn assignment_updates_variable() {
        let (p, errors) = parse("let x = 1; x = 42;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    #[test]
    fn assignment_to_undeclared_errors() {
        let (p, _e) = parse("x = 42;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(
            err.contains("Cannot assign to undeclared variable"),
            "err was: {}",
            err
        );
    }

    #[test]
    fn error_message_includes_line_and_column() {
        // RES-005: errors carry `line:col:` prefix from the Lexer.
        let src = "fn main() {\n    let = 1;\n}";
        let (_p, errors) = parse(src);
        assert!(!errors.is_empty(), "expected an error for missing ident");
        // The missing identifier is on line 2.
        let first = &errors[0];
        assert!(
            first.starts_with("2:"),
            "expected error prefixed with '2:', got: {}",
            first
        );
    }

    #[test]
    fn lexer_tracks_line_across_newlines() {
        let mut lex = Lexer::new("let x = 1;\nlet y = 2;".to_string());
        let _ = lex.next_token(); // let (line 1)
        let _ = lex.next_token(); // x
        let _ = lex.next_token(); // =
        let _ = lex.next_token(); // 1
        let _ = lex.next_token(); // ;
        let _ = lex.next_token(); // let (line 2)
        assert_eq!(lex.last_token_line, 2, "second `let` should be on line 2");
    }

    #[test]
    fn int_modulo() {
        let (p, _e) = parse("let x = 7 % 3;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, 1),
            other => panic!("expected Int(1), got {:?}", other),
        }
    }

    #[test]
    fn int_modulo_by_zero_errors() {
        let (p, _e) = parse("let x = 5 % 0;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("Modulo by zero"), "err: {}", err);
    }

    #[test]
    fn prefix_bang_evaluates() {
        let (p, _e) = parse("let x = !true;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Bool(b) => assert!(!b),
            other => panic!("expected Bool(false), got {:?}", other),
        }
    }

    #[test]
    fn prefix_minus_evaluates() {
        let (p, _e) = parse("let x = -5;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("x").unwrap() {
            Value::Int(n) => assert_eq!(n, -5),
            other => panic!("expected Int(-5), got {:?}", other),
        }
    }

    #[test]
    fn prefix_bang_on_identifier() {
        let (p, errors) = parse("let t = true; let f = !t;");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        match interp.env.get("f").unwrap() {
            Value::Bool(b) => assert!(!b),
            other => panic!("expected Bool(false), got {:?}", other),
        }
    }

    #[test]
    fn parser_if_with_infix_condition() {
        // RES-014: `if call_expr() < 0.5 { ... }` used to report
        // "Expected '{' after if condition, found FloatLiteral(0.5)"
        // because parse_expression left current_token on the last
        // literal of the condition.
        let (_p, errors) = parse("fn f() { if 1 < 2 { let x = 1; } }");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn parser_if_with_function_call_comparison() {
        let (_p, errors) = parse("fn f() { if add(1, 2) == 3 { let x = 1; } }");
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn parser_recovers_from_missing_if_brace() {
        // RES-009: before this ticket, `if x == 1 foo();` (no `{` after
        // the condition) panicked the whole interpreter. Now it should
        // record a parse error and keep going.
        let (_program, errors) = parse("fn f() { if x == 1 x; }");
        assert!(
            !errors.is_empty(),
            "expected a parse error for missing `{{`"
        );
        assert!(
            errors.iter().any(|e| e.contains("Expected '{'")),
            "expected a message naming the missing brace, got {:?}",
            errors
        );
    }

    #[test]
    fn parser_accepts_bare_return() {
        // RES-011: `return;` used to panic on unwrap().
        let (program, errors) = parse("fn foo() { return; }");
        assert!(errors.is_empty(), "errors: {:?}", errors);
        match program {
            Node::Program(stmts) => match &stmts[0].node {
                Node::Function { body, .. } => match body.as_ref() {
                    Node::Block { stmts: inner, .. } => match &inner[0] {
                        Node::ReturnStatement { value, .. } => assert!(value.is_none()),
                        other => panic!("expected ReturnStatement, got {:?}", other),
                    },
                    other => panic!("expected Block, got {:?}", other),
                },
                other => panic!("expected Function, got {:?}", other),
            },
            other => panic!("expected Program, got {:?}", other),
        }
    }

    #[test]
    fn parser_accepts_return_with_value() {
        let (_program, errors) = parse("fn foo() { return 42; }");
        assert!(errors.is_empty(), "errors: {:?}", errors);
    }

    #[test]
    fn interpreter_evaluates_let_and_arithmetic() {
        let (program, errors) = parse("let x = 40; let y = x + 2;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&program).expect("eval should succeed");
        match interp.env.get("y").expect("y defined") {
            Value::Int(v) => assert_eq!(v, 42),
            other => panic!("expected Int(42), got {:?}", other),
        }
    }

    // ---------- RES-055: type-preserving math builtins ----------

    #[test]
    fn floor_of_int_returns_int() {
        match builtin_floor(&[Value::Int(7)]).unwrap() {
            Value::Int(7) => {}
            other => panic!("expected Int(7), got {:?}", other),
        }
    }

    #[test]
    fn floor_of_float_still_returns_float() {
        match builtin_floor(&[Value::Float(3.7)]).unwrap() {
            Value::Float(f) => assert_eq!(f, 3.0),
            other => panic!("expected Float(3.0), got {:?}", other),
        }
    }

    #[test]
    fn ceil_of_negative_int_returns_int() {
        match builtin_ceil(&[Value::Int(-3)]).unwrap() {
            Value::Int(-3) => {}
            other => panic!("expected Int(-3), got {:?}", other),
        }
    }

    #[test]
    fn ceil_of_float_still_returns_float() {
        match builtin_ceil(&[Value::Float(-2.4)]).unwrap() {
            Value::Float(f) => assert_eq!(f, -2.0),
            other => panic!("expected Float(-2.0), got {:?}", other),
        }
    }

    #[test]
    fn pow_of_int_int_returns_int() {
        match builtin_pow(&[Value::Int(2), Value::Int(10)]).unwrap() {
            Value::Int(1024) => {}
            other => panic!("expected Int(1024), got {:?}", other),
        }
    }

    #[test]
    fn pow_int_overflow_is_clean_error() {
        // 2^63 overflows i64.
        let err = builtin_pow(&[Value::Int(2), Value::Int(63)])
            .expect_err("must overflow");
        assert!(err.contains("overflow"), "error should mention overflow: {}", err);
    }

    #[test]
    fn pow_with_negative_int_exponent_errors() {
        let err = builtin_pow(&[Value::Int(2), Value::Int(-1)])
            .expect_err("negative exp must error for int base");
        assert!(
            err.contains("negative exponent"),
            "error should mention negative exponent: {}",
            err
        );
    }

    #[test]
    fn pow_keeps_float_behavior_when_either_arg_is_float() {
        match builtin_pow(&[Value::Float(2.0), Value::Int(3)]).unwrap() {
            Value::Float(f) => assert_eq!(f, 8.0),
            other => panic!("expected Float(8.0), got {:?}", other),
        }
        match builtin_pow(&[Value::Int(2), Value::Float(3.0)]).unwrap() {
            Value::Float(f) => assert_eq!(f, 8.0),
            other => panic!("expected Float(8.0), got {:?}", other),
        }
    }

    #[test]
    fn sqrt_of_int_still_returns_float() {
        // RES-055: sqrt deliberately UNCHANGED — irrational results
        // are the norm, so Float is the right return type.
        match builtin_sqrt(&[Value::Int(4)]).unwrap() {
            Value::Float(f) => assert_eq!(f, 2.0),
            other => panic!("expected Float(2.0), got {:?}", other),
        }
    }

    #[test]
    fn abs_min_max_remain_type_preserving() {
        // Sanity check that our changes didn't regress the builtins
        // that were already type-preserving.
        match builtin_abs(&[Value::Int(-5)]).unwrap() {
            Value::Int(5) => {}
            other => panic!("abs(Int(-5)): expected Int(5), got {:?}", other),
        }
        match builtin_min(&[Value::Int(2), Value::Int(7)]).unwrap() {
            Value::Int(2) => {}
            other => panic!("min: expected Int(2), got {:?}", other),
        }
        match builtin_max(&[Value::Int(2), Value::Int(7)]).unwrap() {
            Value::Int(7) => {}
            other => panic!("max: expected Int(7), got {:?}", other),
        }
    }

    // ---------- RES-034: nested index assignment ----------

    /// Read `m[i][j]` and assert it is `Value::Int(expected)`.
    fn nested_int(m: &Value, i: usize, j: usize) -> i64 {
        let Value::Array(rows) = m else { panic!("expected outer Array, got {:?}", m); };
        let Value::Array(row) = &rows[i] else { panic!("expected inner Array at row {}, got {:?}", i, rows[i]); };
        match &row[j] {
            Value::Int(v) => *v,
            other => panic!("expected Int at [{}][{}], got {:?}", i, j, other),
        }
    }

    #[test]
    fn nested_index_assignment_writes_leaf_cell() {
        // RES-034: a[i][j] = v should mutate exactly the addressed cell.
        let (p, errors) = parse("let m = [[1, 2], [3, 4]]; m[1][0] = 9;");
        assert!(errors.is_empty(), "{:?}", errors);
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let m = interp.env.get("m").unwrap();
        assert_eq!(nested_int(&m, 1, 0), 9);
    }

    #[test]
    fn nested_index_assignment_leaves_siblings_untouched() {
        // RES-034: writing m[0][1] must not disturb m[0][0], m[1][*], etc.
        let (p, _e) = parse("let m = [[1, 2], [3, 4]]; m[0][1] = 9;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let m = interp.env.get("m").unwrap();
        assert_eq!(nested_int(&m, 0, 0), 1);
        assert_eq!(nested_int(&m, 0, 1), 9);
        assert_eq!(nested_int(&m, 1, 0), 3);
        assert_eq!(nested_int(&m, 1, 1), 4);
    }

    #[test]
    fn nested_index_assignment_outer_out_of_bounds_errors_cleanly() {
        // RES-034: outer index out of range must be a clean error,
        // not a panic. Bounds error names the depth.
        let (p, _e) = parse("let m = [[1, 2], [3, 4]]; m[2][0] = 9;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of bounds"), "got: {}", err);
        assert!(err.contains("dim 1"), "should name outer dim: {}", err);
    }

    #[test]
    fn nested_index_assignment_inner_out_of_bounds_errors_cleanly() {
        // RES-034: inner index out of range names the inner dim so the
        // user can tell which dimension blew up.
        let (p, _e) = parse("let m = [[1, 2]]; m[0][5] = 9;");
        let mut interp = Interpreter::new();
        let err = interp.eval(&p).unwrap_err();
        assert!(err.contains("out of bounds"), "got: {}", err);
        assert!(err.contains("dim 2"), "should name inner dim: {}", err);
    }

    #[test]
    fn three_deep_nested_index_assignment() {
        // RES-034: descent works at arbitrary depth, not just 2.
        let (p, _e) = parse("let m = [[[1], [2]], [[3], [4]]]; m[1][0][0] = 99;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let m = interp.env.get("m").unwrap();
        let Value::Array(outer) = &m else { panic!("outer"); };
        let Value::Array(mid) = &outer[1] else { panic!("mid"); };
        let Value::Array(leaf) = &mid[0] else { panic!("leaf"); };
        assert!(matches!(leaf[0], Value::Int(99)));
    }

    // ---------- RES-077: Program statements carry Span ----------

    #[test]
    fn typecheck_error_includes_file_line_col_prefix() {
        // RES-080: when the offending top-level statement is on line 2,
        // the error string should be prefixed with `<file>:2:<col>:`.
        // Use the existing typecheck rule that rejects mismatched
        // type annotations (`let x: int = "hi";` triggers it).
        let src = "let ok = 1;\nlet bad: int = \"hi\";";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program_with_source(&program, "scratch.rs")
            .expect_err("type checker must reject the second statement");
        assert!(
            err.starts_with("scratch.rs:2:"),
            "expected file:line prefix, got: {}",
            err
        );
        // The original message must still be present after the prefix.
        assert!(
            err.contains("let bad: int") || err.contains("string"),
            "expected original type-error wording in: {}",
            err
        );
    }

    #[test]
    fn check_program_legacy_shim_uses_unknown_source() {
        // RES-080 backward-compat: callers that haven't migrated to
        // check_program_with_source still get a helpful message —
        // just prefixed with `<unknown>` instead of a real path.
        let src = "let bad: int = \"hi\";";
        let (program, _errors) = parse(src);
        let err = typechecker::TypeChecker::new()
            .check_program(&program)
            .unwrap_err();
        assert!(
            err.starts_with("<unknown>:1:"),
            "legacy shim should use <unknown> prefix, got: {}",
            err
        );
    }

    #[test]
    fn function_declarations_carry_spans_per_source_line() {
        // RES-088: a 2-fn source produces two Function nodes whose
        // spans reflect the source line the `fn` keyword sits on.
        let src = "fn one() { return 1; }\nfn two() { return 2; }";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let Node::Program(stmts) = &program else { panic!() };
        let Node::Function { span: s0, .. } = &stmts[0].node else {
            panic!("expected Function for stmt 0");
        };
        let Node::Function { span: s1, .. } = &stmts[1].node else {
            panic!("expected Function for stmt 1");
        };
        assert!(s0.start.line >= 1, "fn 1 span: {:?}", s0);
        assert!(s1.start.line >= 2, "fn 2 span: {:?}", s1);
        assert!(
            s1.start.line > s0.start.line,
            "expected fn 2 line ({}) > fn 1 line ({})",
            s1.start.line, s0.start.line
        );
    }

    #[test]
    fn block_and_expression_statement_carry_spans() {
        // RES-087: Block and ExpressionStatement are struct variants
        // now. Parse a fn body + confirm both have populated spans.
        let (program, errors) = parse("fn f() { let x = 1; let y = 2; }");
        assert!(errors.is_empty());
        let Node::Program(stmts) = &program else { panic!() };
        let Node::Function { body, .. } = &stmts[0].node else { panic!() };
        let Node::Block { stmts: inner, span: block_span } = body.as_ref() else {
            panic!("expected Block");
        };
        assert_eq!(inner.len(), 2);
        assert!(block_span.start.line >= 1, "block span: {:?}", block_span);

        // ExpressionStatement: `1 + 2;` at top level
        let (program, _) = parse("1 + 2;");
        let Node::Program(stmts) = &program else { panic!() };
        let Node::ExpressionStatement { expr: _, span } = &stmts[0].node else {
            panic!("expected ExpressionStatement");
        };
        assert!(span.start.line >= 1, "expr-stmt span: {:?}", span);
    }

    #[test]
    fn array_literal_and_try_carry_spans() {
        // RES-086: ArrayLiteral and TryExpression are struct variants
        // now. Confirm both parse sites populate their spans.
        let (program, errors) = parse("[1, 2, 3];");
        assert!(errors.is_empty());
        let Node::Program(stmts) = &program else { panic!() };
        let Node::ExpressionStatement { expr, .. } = &stmts[0].node else { panic!() };
        let Node::ArrayLiteral { items, span } = expr.as_ref() else {
            panic!("expected ArrayLiteral, got {:?}", expr);
        };
        assert_eq!(items.len(), 3);
        assert!(span.start.line >= 1, "array span: {:?}", span);

        // TryExpression: `ok(1)?`
        let (program, _) = parse("fn f() { let x = ok(1)?; return x; }");
        // Walk into the fn body to find the `?` expression.
        let Node::Program(stmts) = &program else { panic!() };
        let Node::Function { body, .. } = &stmts[0].node else { panic!() };
        let Node::Block { stmts: inner, .. } = body.as_ref() else { panic!() };
        let Node::LetStatement { value, .. } = &inner[0] else { panic!() };
        let Node::TryExpression { span, .. } = value.as_ref() else {
            panic!("expected TryExpression, got {:?}", value);
        };
        assert!(span.start.line >= 1, "try span: {:?}", span);
    }

    #[test]
    fn index_and_field_expressions_carry_spans() {
        // RES-085: `a[0]` builds an IndexExpression whose span lands
        // on the `[`. `a.b` builds a FieldAccess whose span lands on
        // the `.`. Both should have start.line >= 1 when parsed from
        // real source.
        let (program, errors) = parse("let a = [1, 2]; a[0]; let b = a; b.len;");
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let Node::Program(stmts) = &program else { panic!() };
        // stmt 1: `a[0];` expression statement wrapping IndexExpression
        let Node::ExpressionStatement { expr, .. } = &stmts[1].node else { panic!() };
        let Node::IndexExpression { span, .. } = expr.as_ref() else {
            panic!("expected IndexExpression");
        };
        assert!(span.start.line >= 1, "index span: {:?}", span);
        // stmt 3: `b.len;` expression statement wrapping FieldAccess
        let Node::ExpressionStatement { expr, .. } = &stmts[3].node else { panic!() };
        let Node::FieldAccess { span, .. } = expr.as_ref() else {
            panic!("expected FieldAccess");
        };
        assert!(span.start.line >= 1, "field span: {:?}", span);
    }

    #[test]
    fn infix_expression_carries_operator_span() {
        // RES-084: the operator's span lands on the InfixExpression
        // node, NOT a default zero. `1 + 2` is a single line of
        // source so start.line should be >= 1.
        let (program, errors) = parse("1 + 2;");
        assert!(errors.is_empty());
        let Node::Program(stmts) = &program else { panic!() };
        let Node::ExpressionStatement { expr, .. } = &stmts[0].node else {
            panic!("expected ExpressionStatement, got {:?}", stmts[0].node);
        };
        let Node::InfixExpression { operator, span, .. } = expr.as_ref() else {
            panic!("expected InfixExpression, got {:?}", expr);
        };
        assert_eq!(operator, "+");
        assert!(span.start.line >= 1, "infix span: {:?}", span);
    }

    #[test]
    fn prefix_and_call_expressions_carry_spans() {
        // RES-084: prefix `!x` and call `f()` both record their
        // operator/parenthesis location.
        let (program, _) = parse("fn f() { return 1; }\n!true;\nf();");
        let Node::Program(stmts) = &program else { panic!() };
        // stmt 0 is the fn decl; stmt 1 is the !true expression
        let Node::ExpressionStatement { expr, .. } = &stmts[1].node else { panic!() };
        let Node::PrefixExpression { operator, span, .. } = expr.as_ref() else { panic!() };
        assert_eq!(operator, "!");
        assert!(span.start.line >= 1);
        // stmt 2 is the call f()
        let Node::ExpressionStatement { expr, .. } = &stmts[2].node else { panic!() };
        let Node::CallExpression { span, .. } = expr.as_ref() else { panic!() };
        assert!(span.start.line >= 1);
    }

    #[test]
    fn let_statement_spans_track_source_line() {
        // RES-079: the inner LetStatement span matches the line of
        // its source origin. Statement 1 is on line 1, statement 2
        // on line 2.
        let src = "let x = 1;\nlet y = 2;";
        let (program, errors) = parse(src);
        assert!(errors.is_empty());
        let Node::Program(stmts) = &program else { panic!("expected Program"); };
        let Node::LetStatement { span: s0, .. } = &stmts[0].node else { panic!(); };
        let Node::LetStatement { span: s1, .. } = &stmts[1].node else { panic!(); };
        assert!(s0.start.line >= 1, "s0 line: {}", s0.start.line);
        assert!(s1.start.line >= 2, "s1 line: {}", s1.start.line);
        assert!(s1.start.line > s0.start.line);
    }

    #[test]
    fn literal_and_identifier_nodes_carry_non_default_spans() {
        // RES-078: leaf nodes (IntegerLiteral here + Identifier for
        // `x`) come back with populated Span fields so the typechecker
        // and verifier can attribute errors to them.
        let src = "let x = 42;";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let Node::Program(stmts) = &program else {
            panic!("expected Program");
        };
        let Node::LetStatement { value, .. } = &stmts[0].node else {
            panic!("expected LetStatement");
        };
        let Node::IntegerLiteral { value: lit, span } = value.as_ref() else {
            panic!("expected IntegerLiteral");
        };
        assert_eq!(*lit, 42);
        assert!(span.start.line >= 1, "int literal span.start.line = {}", span.start.line);
    }

    #[test]
    fn undefined_variable_error_includes_line_col() {
        // RES-078: when the typechecker rejects an undefined name,
        // the Identifier's span should surface in the error message.
        let (program, _) = parse("let x = undefined_thing;");
        let mut tc = typechecker::TypeChecker::new();
        let err = tc.check_program(&program).expect_err("must reject undefined");
        // check_program goes through the statement-level prefix
        // (RES-080) too, so the error has two file:line:col segments.
        // We just need the identifier-level `at N:M` part to appear.
        assert!(
            err.contains("undefined_thing") && err.contains("at "),
            "expected `at LINE:COL` from RES-078 identifier span; got: {}",
            err
        );
    }

    #[test]
    fn program_statements_carry_non_default_spans() {
        // RES-077: every top-level statement comes back as a
        // Spanned<Node> with a populated Span. Use two statements
        // separated by a newline so we can assert the second's start
        // line is strictly later than the first's.
        let src = "let x = 1;\nlet y = 2;";
        let (program, errors) = parse(src);
        assert!(errors.is_empty(), "parse errors: {:?}", errors);
        let Node::Program(stmts) = &program else {
            panic!("expected Program, got {:?}", program);
        };
        assert_eq!(stmts.len(), 2);
        let s0 = &stmts[0];
        let s1 = &stmts[1];
        // Spans must be non-default — a default Span has line 0.
        assert!(s0.span.start.line >= 1, "stmt0 start line: {:?}", s0.span);
        assert!(s1.span.start.line >= 1, "stmt1 start line: {:?}", s1.span);
        // And ordered: stmt 1 starts on a later line than stmt 0.
        assert!(
            s1.span.start.line > s0.span.start.line,
            "expected line order, got s0={:?} s1={:?}",
            s0.span, s1.span
        );
        // The inner node still has its existing shape.
        assert!(matches!(s0.node, Node::LetStatement { .. }));
    }

    #[test]
    fn single_dim_index_assignment_still_works() {
        // RES-034 regression: the single-index path must still work
        // (it's the same code path now, and we shouldn't break the
        // common case while generalizing).
        let (p, _e) = parse("let a = [1, 2, 3]; a[1] = 42;");
        let mut interp = Interpreter::new();
        interp.eval(&p).unwrap();
        let Value::Array(items) = interp.env.get("a").unwrap() else {
            panic!("expected Array");
        };
        assert!(matches!(items[1], Value::Int(42)));
    }

    // --- RES-116: interpreter runtime errors carry line:col: spans ---

    /// Execute `src` through the tree-walker and return the `Err`
    /// string. Panics if the program runs without error. Shared by
    /// the three runtime-error-class tests below.
    fn interp_err(src: &str) -> String {
        let (program, parser_errs) = parse(src);
        assert!(
            parser_errs.is_empty(),
            "test source failed to parse: {:?}",
            parser_errs
        );
        let mut interp = Interpreter::new();
        match interp.eval(&program) {
            Ok(_) => panic!("expected runtime error, got Ok"),
            Err(e) => e,
        }
    }

    #[test]
    fn runtime_error_divide_by_zero_has_line_col_prefix() {
        // Line 1: function def; line 2: divide; the call happens on
        // line 5. The driver's span decoration reports the TOP-LEVEL
        // statement's line, so we expect line 5 (the `boom(0);` call).
        let src = "fn boom(int n) {\n    let r = 100 / n;\n    return r;\n}\nboom(0);";
        let e = interp_err(src);
        assert!(
            has_line_col_prefix(&e),
            "expected `line:col:` prefix, got: {:?}",
            e
        );
        assert!(e.contains("Division by zero"), "got: {:?}", e);
        assert!(e.starts_with("5:"), "expected line 5 prefix, got: {:?}", e);
    }

    #[test]
    fn runtime_error_array_oob_has_line_col_prefix() {
        // `let a = [1];` on line 1, OOB read on line 2.
        let src = "let a = [1];\nlet b = a[7];";
        let e = interp_err(src);
        assert!(
            has_line_col_prefix(&e),
            "expected `line:col:` prefix, got: {:?}",
            e
        );
        assert!(e.starts_with("2:"), "expected line 2 prefix, got: {:?}", e);
    }

    #[test]
    fn runtime_error_unknown_function_has_line_col_prefix() {
        // Unknown function call on line 3.
        let src = "let a = 1;\nlet b = 2;\nnot_a_real_fn(a, b);";
        let e = interp_err(src);
        assert!(
            has_line_col_prefix(&e),
            "expected `line:col:` prefix, got: {:?}",
            e
        );
        assert!(e.starts_with("3:"), "expected line 3 prefix, got: {:?}", e);
    }

    #[test]
    fn has_line_col_prefix_accepts_decimal_forms() {
        assert!(has_line_col_prefix("1:1: foo"));
        assert!(has_line_col_prefix("12:34: bar"));
    }

    #[test]
    fn has_line_col_prefix_rejects_non_span_forms() {
        assert!(!has_line_col_prefix("Runtime error: division by zero"));
        assert!(!has_line_col_prefix("abc:1:1: bad"));
        assert!(!has_line_col_prefix(":1: bad"));
        assert!(!has_line_col_prefix("1:: bad"));
        assert!(!has_line_col_prefix(""));
    }

    #[test]
    fn format_interpreter_error_shapes_decorated_msg() {
        let out = format_interpreter_error("/tmp/foo.rs", "2:7: Division by zero");
        assert_eq!(out, "/tmp/foo.rs:2:7: Runtime error: Division by zero");
    }

    #[test]
    fn format_interpreter_error_falls_back_when_undecorated() {
        let out = format_interpreter_error("/tmp/foo.rs", "something went wrong");
        assert_eq!(out, "Runtime error: something went wrong");
    }

    // --- RES-113: shebang line at start of file ---

    #[test]
    fn lexer_shebang_line_ignored() {
        // A leading shebang line is silently consumed; the first
        // real token (`println`) lands on line 2, col 1.
        let src = "#!/usr/bin/env resilient\nprintln(\"ok\");";
        let mut lex = Lexer::new(src.to_string());
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Identifier(ref n) => assert_eq!(n, "println"),
            other => panic!("expected Identifier(println), got {:?}", other),
        }
        assert_eq!(span.start.line, 2);
        assert_eq!(span.start.column, 1);
    }

    #[test]
    fn lexer_shebang_not_at_start_errors() {
        // `#!` anywhere other than byte 0 must lex as an Unknown
        // `#` token — no free comment syntax from this change.
        let src = "println(\"hi\");\n#!/bin/sh";
        let mut lex = Lexer::new(src.to_string());
        // Drain the legitimate prefix.
        let mut saw_unknown_hash = false;
        loop {
            let tok = lex.next_token();
            if matches!(tok, Token::Eof) {
                break;
            }
            if matches!(tok, Token::Unknown('#')) {
                saw_unknown_hash = true;
            }
        }
        assert!(
            saw_unknown_hash,
            "expected Token::Unknown('#') somewhere in the stream"
        );
    }

    #[test]
    fn lexer_empty_shebang_line() {
        // `#!\n` (no path) followed by code: still consumed. Real
        // token lands on line 2.
        let src = "#!\nprintln(\"ok\");";
        let mut lex = Lexer::new(src.to_string());
        let (tok, span) = lex.next_token_with_span();
        match tok {
            Token::Identifier(ref n) => assert_eq!(n, "println"),
            other => panic!("expected Identifier(println), got {:?}", other),
        }
        assert_eq!(span.start.line, 2);
        assert_eq!(span.start.column, 1);
    }

    #[test]
    fn lexer_shebang_only_no_trailing_newline() {
        // File is just `#!/usr/bin/env resilient` (no trailing
        // newline, no code). Shebang consumed, next token is Eof.
        let src = "#!/usr/bin/env resilient";
        let mut lex = Lexer::new(src.to_string());
        let tok = lex.next_token();
        assert!(matches!(tok, Token::Eof), "got {:?}", tok);
    }

    // --- RES-114: ASCII-only identifier policy ---

    #[test]
    fn lexer_rejects_cyrillic_identifier() {
        // Cyrillic `кафа` and Latin `kafa` look visually identical
        // in most fonts — the ASCII-only policy is the defense
        // against that class of homoglyph attack.
        let src = "let кафа = 1;";
        let mut lex = Lexer::new(src.to_string());
        let mut saw_non_ascii = false;
        loop {
            let tok = lex.next_token();
            if matches!(tok, Token::Eof) {
                break;
            }
            if let Token::Unknown(c) = tok
                && !c.is_ascii()
            {
                saw_non_ascii = true;
            }
        }
        assert!(
            saw_non_ascii,
            "expected non-ASCII char to surface as Token::Unknown"
        );
    }

    #[test]
    fn lexer_rejects_mixed_latin_greek() {
        // `Αlpha` at the statement head — uppercase Greek Alpha
        // (U+0391) then Latin `lpha`. The Greek start char falls
        // through to parse_statement's `Token::Unknown` arm,
        // which routes it through the dedicated non-ASCII
        // diagnostic. (Bare `let Αlpha` would hit the let-parser's
        // "expected identifier" error first; using a statement-
        // level position exercises the path the policy is actually
        // designed for.)
        let src = "Αlpha;";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("non-ASCII")),
            "expected non-ASCII diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn lexer_accepts_underscored_names() {
        // Plain ASCII underscores + digits + letters continue to
        // lex as a single Identifier (sanity check that the policy
        // tightening didn't regress the common case).
        let src = "let _leading_underscore_123 = 1;";
        let (_program, errs) = parse(src);
        assert!(
            errs.is_empty(),
            "expected no parse errors for ASCII ident, got: {:?}",
            errs
        );
    }

    // --- RES-118: parser "expected one of …" hints ---

    #[test]
    fn expected_hint_multi_token_alternatives() {
        // `{ x: 1 y: 2 }` — missing comma between fields inside a
        // struct literal. The parser now points at `y` as the
        // offending token and lists both `,` and `}` as valid
        // alternatives via `format_expected`.
        let src = "struct Point { int x, int y, } let p = new Point { x: 1 y: 2 };";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("expected one of")
                && e.contains("`,`")
                && e.contains("`}`")),
            "expected multi-alternative diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn expected_hint_singleton_specializes_to_singular_form() {
        // Singleton: `format_expected(&["`,`"], got)` must render
        // `expected `,`, got …` — not `expected one of `,`, got …`.
        // The generator specializes on the singleton case so the
        // common "missing X" message reads naturally.
        let s = format_expected(&["`,`"], "`)`");
        assert_eq!(s, "expected `,`, got `)`");
    }

    #[test]
    fn expected_hint_caps_long_lists_with_ellipsis() {
        // Slices longer than 5 entries truncate with `…`.
        let s = format_expected(
            &["`a`", "`b`", "`c`", "`d`", "`e`", "`f`", "`g`"],
            "`x`",
        );
        assert!(
            s.starts_with("expected one of `a`, `b`, `c`, `d`, `e`, …"),
            "unexpected shape: {}",
            s
        );
        assert!(s.ends_with(", got `x`"), "unexpected tail: {}", s);
    }

    // --- RES-123: optional (inferred) return type annotations ---

    // --- RES-141: process-wide live-block telemetry ---

    #[test]
    fn live_total_retries_zero_arity() {
        // Zero-arg contract.
        let err = builtin_live_total_retries(&[Value::Int(0)]).unwrap_err();
        assert!(
            err.contains("expected 0 arguments"),
            "got: {}",
            err
        );
        let err = builtin_live_total_exhaustions(&[Value::Int(0)]).unwrap_err();
        assert!(
            err.contains("expected 0 arguments"),
            "got: {}",
            err
        );
    }

    #[test]
    fn live_total_counters_advance_on_retries_and_exhaustions() {
        // Deltas, not absolutes — the counters are process-wide
        // atomics and other tests running in parallel can bump
        // them. We take a before/after snapshot around this test's
        // own live-block work and assert the diff.
        use std::sync::atomic::Ordering::Relaxed;
        let before_retries = LIVE_TOTAL_RETRIES.load(Relaxed);
        let before_exhaust = LIVE_TOTAL_EXHAUSTIONS.load(Relaxed);

        // Workload: two nested always-fail blocks. Inner + outer
        // each exhaust their 3-retry budgets → 3 × 3 = 9 inner
        // invocations, each contributing one retry (except the
        // successful terminators, which don't exist here because
        // everything fails). Let's count carefully:
        //
        //   - Outer attempt 1 runs inner. Inner fails 3 times.
        //     Inner retries 2 (from fail 1→2 and 2→3); inner
        //     exhausts (1 exhaustion). Outer sees the escalated
        //     error → outer retries (1 outer retry).
        //   - Outer attempt 2: same pattern. 2 inner retries + 1
        //     inner exhaustion + 1 outer retry.
        //   - Outer attempt 3: 2 inner retries + 1 inner
        //     exhaustion. Outer retry counter hits 3 → outer
        //     exhausts (1 exhaustion), no outer retry.
        //
        // Totals: inner_retries = 3 * 2 = 6. Outer retries = 2.
        // Inner exhaustions = 3. Outer exhaustions = 1.
        // Total retries = 8. Total exhaustions = 4.
        let src = "\
            fn always_fail() {\n\
                assert(false, \"forced\");\n\
                return 0;\n\
            }\n\
            fn main(int _d) {\n\
                live {\n\
                    live { let r = always_fail(); }\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let _ = interp.eval(&program); // expect exhaust; don't fail the test on it

        let retries = LIVE_TOTAL_RETRIES.load(Relaxed) - before_retries;
        let exhaust = LIVE_TOTAL_EXHAUSTIONS.load(Relaxed) - before_exhaust;
        // Lower-bound assertions — other tests running in parallel
        // bump the same process-wide atomics, inflating our delta.
        // The test's own workload contributes at least 8 retries
        // and 4 exhaustions (inner 6 + outer 2 retries; inner 3 +
        // outer 1 exhaustions), so >= is robust.
        assert!(
            retries >= 8,
            "expected >= 8 retries (this test contributes 8), got {}",
            retries
        );
        assert!(
            exhaust >= 4,
            "expected >= 4 exhaustions (this test contributes 4), got {}",
            exhaust
        );
    }

    // --- RES-140: nested live-block escalation ---

    #[test]
    fn nested_live_inner_exhaustion_counts_as_one_outer_retry() {
        // Outer retries → inner re-runs to exhaustion each time.
        // With MAX_RETRIES=3 per level and `always_fail`, total
        // inner invocations = outer_attempts * inner_attempts =
        // 3 * 3 = 9. A `static let` counter confirms.
        let src = "\
            static let inner_calls = 0;\n\
            fn always_fail() {\n\
                inner_calls = inner_calls + 1;\n\
                assert(false, \"inner\");\n\
                return 0;\n\
            }\n\
            fn main(int _d) {\n\
                live {\n\
                    live { let r = always_fail(); }\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp
            .eval(&program)
            .expect_err("outer live block should eventually exhaust");
        // Error shape: `Live block failed after 3 attempts (retry depth: 1): Live block failed after 3 attempts (retry depth: 2): ...`
        assert!(err.contains("Live block failed after 3 attempts"), "got: {}", err);
        assert!(err.contains("retry depth: 1"), "outer level note missing: {}", err);
        assert!(err.contains("retry depth: 2"), "inner level note missing: {}", err);
        // 3 outer * 3 inner = 9 inner invocations.
        match interp.statics.borrow().get("inner_calls").expect("static counter") {
            Value::Int(n) => assert_eq!(*n, 9, "expected 9 inner invocations, got {}", n),
            other => panic!("expected Int, got {:?}", other),
        }
    }

    #[test]
    fn nested_live_retries_reports_innermost_counter() {
        // `live_retries()` inside the inner block reads 0, 1, 2
        // per outer attempt — independent of the outer's counter.
        // We capture the innermost reads on the first outer
        // attempt only (inner exhaustion then escalates) to keep
        // the test deterministic.
        let src = "\
            static let inner_fails = 3;\n\
            static let seen = [];\n\
            fn always_fail() {\n\
                seen = push(seen, live_retries());\n\
                inner_fails = inner_fails - 1;\n\
                assert(false, \"inner\");\n\
                return 0;\n\
            }\n\
            fn main(int _d) {\n\
                live {\n\
                    live { let r = always_fail(); }\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let _ = interp.eval(&program); // exhausts; ignore error
        match interp.statics.borrow().get("seen").expect("static seen") {
            Value::Array(items) => {
                let ns: Vec<i64> = items
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => *n,
                        other => panic!("non-int in seen: {:?}", other),
                    })
                    .collect();
                // Inner ran 9 times (3 outer × 3 inner). The inner
                // counter resets to 0 at every new inner-block
                // entry, so the sequence is 0,1,2 repeated three
                // times.
                assert_eq!(ns, vec![0, 1, 2, 0, 1, 2, 0, 1, 2], "got {:?}", ns);
            }
            other => panic!("expected Array, got {:?}", other),
        }
    }

    // --- RES-139: live exponential backoff ---

    #[test]
    fn backoff_delay_ms_caps_at_max() {
        let cfg = BackoffConfig { base_ms: 1, factor: 2, max_ms: 100 };
        assert_eq!(cfg.delay_ms(0), 1);    // 1 * 2^0 = 1
        assert_eq!(cfg.delay_ms(1), 2);    // 1 * 2^1 = 2
        assert_eq!(cfg.delay_ms(5), 32);   // 1 * 2^5 = 32
        assert_eq!(cfg.delay_ms(7), 100);  // 1 * 2^7 = 128, capped at 100
        assert_eq!(cfg.delay_ms(30), 100); // huge growth still capped
    }

    #[test]
    fn backoff_delay_ms_saturates_without_overflow() {
        // `saturating_pow` / `saturating_mul` guard against `u64`
        // wrap on intentionally aggressive values; the cap still
        // holds.
        let cfg = BackoffConfig { base_ms: 1_000_000, factor: 10, max_ms: 50 };
        assert_eq!(cfg.delay_ms(63), 50);
    }

    #[test]
    fn parse_live_backoff_kwargs_populates_config() {
        // All three kwargs explicit.
        let src = "fn main(int _d) { live backoff(base_ms=7, factor=3, max_ms=250) { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        // Dig out the backoff config from the parsed main fn's
        // body block's first LiveBlock.
        let cfg = find_first_live_backoff(&program).expect("live block with backoff");
        assert_eq!(cfg.base_ms, 7);
        assert_eq!(cfg.factor, 3);
        assert_eq!(cfg.max_ms, 250);
    }

    #[test]
    fn parse_live_backoff_defaults_fill_missing_kwargs() {
        // Only factor specified — others fall back to ticket
        // defaults (1 / 2 / 100).
        let src = "fn main(int _d) { live backoff(factor=4) { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let cfg = find_first_live_backoff(&program).expect("live block with backoff");
        assert_eq!(cfg.base_ms, 1);
        assert_eq!(cfg.factor, 4);
        assert_eq!(cfg.max_ms, 100);
    }

    #[test]
    fn parse_live_backoff_factor_over_10_errors() {
        let src = "fn main(int _d) { live backoff(factor=25) { let x = 1; } } main(0);";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("`factor` must be <= 10")),
            "expected factor-cap diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn parse_live_without_backoff_keeps_none() {
        let src = "fn main(int _d) { live { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(
            find_first_live_backoff(&program).is_none(),
            "plain `live` must not carry a BackoffConfig"
        );
    }

    #[test]
    fn backoff_sleeps_between_retries() {
        // With base_ms=20 / factor=2 / max_ms=100 and two forced
        // failures, the total sleep is 20 + 40 = 60 ms. We measure
        // wall-clock from before `eval` to after and require the
        // elapsed >= 60 ms (generous lower bound that avoids test
        // flake from faster-than-promised sleeps — std::thread::sleep
        // only *lower-bounds* the duration).
        let src = "\
            static let fails_left = 2;\n\
            fn maybe_fail() {\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced\");\n\
                }\n\
                return 42;\n\
            }\n\
            fn main(int _d) {\n\
                live backoff(base_ms=20, factor=2, max_ms=100) {\n\
                    let r = maybe_fail();\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let t0 = std::time::Instant::now();
        interp.eval(&program).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed.as_millis() >= 60,
            "expected >= 60ms wall-clock (20 + 40), got {}ms",
            elapsed.as_millis()
        );
    }

    /// Helper: find the `BackoffConfig` attached to the first
    /// `Node::LiveBlock` reached by a depth-first walk of `program`.
    /// Returns `None` if no live block is present or the one found
    /// has `backoff: None`.
    fn find_first_live_backoff(program: &Node) -> Option<BackoffConfig> {
        fn walk(n: &Node) -> Option<BackoffConfig> {
            match n {
                Node::LiveBlock { backoff, .. } => *backoff,
                Node::Program(stmts) => stmts.iter().find_map(|s| walk(&s.node)),
                Node::Function { body, .. } => walk(body),
                Node::Block { stmts, .. } => stmts.iter().find_map(walk),
                Node::IfStatement { consequence, alternative, .. } => walk(consequence).or_else(|| {
                    alternative.as_ref().and_then(|a| walk(a))
                }),
                _ => None,
            }
        }
        walk(program)
    }

    // --- RES-142: live within <duration> wall-clock timeout ---

    /// Mirror of `find_first_live_backoff` for the RES-142 timeout
    /// field — returns the parsed `nanos` of the first LiveBlock's
    /// `within <duration>` clause, or `None` if absent.
    fn find_first_live_timeout_ns(program: &Node) -> Option<u64> {
        fn walk(n: &Node) -> Option<u64> {
            match n {
                Node::LiveBlock { timeout, .. } => timeout.as_ref().and_then(|t| match t.as_ref() {
                    Node::DurationLiteral { nanos, .. } => Some(*nanos),
                    _ => None,
                }),
                Node::Program(stmts) => stmts.iter().find_map(|s| walk(&s.node)),
                Node::Function { body, .. } => walk(body),
                Node::Block { stmts, .. } => stmts.iter().find_map(walk),
                Node::IfStatement { consequence, alternative, .. } => walk(consequence).or_else(|| {
                    alternative.as_ref().and_then(|a| walk(a))
                }),
                _ => None,
            }
        }
        walk(program)
    }

    #[test]
    fn parse_live_within_ms_populates_timeout() {
        let src = "fn main(int _d) { live within 10ms { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert_eq!(find_first_live_timeout_ns(&program), Some(10_000_000));
    }

    #[test]
    fn parse_live_within_each_unit() {
        // ns / us / ms / s — assert the unit table matches the
        // ticket's fixed set.
        for (src_unit, expected_ns) in [
            ("3ns", 3_u64),
            ("4us", 4_000_u64),
            ("5ms", 5_000_000_u64),
            ("2s", 2_000_000_000_u64),
        ] {
            let src = format!(
                "fn main(int _d) {{ live within {} {{ let x = 1; }} }} main(0);",
                src_unit
            );
            let (program, errs) = parse(&src);
            assert!(errs.is_empty(), "parse errors for `{}`: {:?}", src_unit, errs);
            assert_eq!(
                find_first_live_timeout_ns(&program),
                Some(expected_ns),
                "mismatch for unit `{}`",
                src_unit
            );
        }
    }

    #[test]
    fn parse_live_unknown_duration_unit_errors() {
        let src = "fn main(int _d) { live within 10min { let x = 1; } } main(0);";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("Unknown duration unit `min`")),
            "expected unknown-unit diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn parse_live_duration_requires_nonneg_int() {
        let src = "fn main(int _d) { live within -5ms { let x = 1; } } main(0);";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("non-negative integer literal after `within`")),
            "expected integer-literal diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn parse_live_within_both_orders_accepted() {
        // Ticket's Notes: `live backoff(...) within 50ms { }` and
        // `live within 50ms backoff(...) { }` both parse.
        for src in [
            "fn main(int _d) { live backoff(base_ms=5) within 50ms { let x = 1; } } main(0);",
            "fn main(int _d) { live within 50ms backoff(base_ms=5) { let x = 1; } } main(0);",
        ] {
            let (program, errs) = parse(src);
            assert!(errs.is_empty(), "parse errors in `{}`: {:?}", src, errs);
            assert_eq!(find_first_live_timeout_ns(&program), Some(50_000_000));
            assert_eq!(
                find_first_live_backoff(&program).map(|c| c.base_ms),
                Some(5),
                "backoff lost in `{}`",
                src
            );
        }
    }

    #[test]
    fn parse_live_duplicate_within_errors() {
        let src = "fn main(int _d) { live within 10ms within 20ms { let x = 1; } } main(0);";
        let (_program, errs) = parse(src);
        assert!(
            errs.iter().any(|e| e.contains("duplicate `within")),
            "expected duplicate-within diagnostic, got: {:?}",
            errs
        );
    }

    #[test]
    fn parse_live_without_within_keeps_none() {
        let src = "fn main(int _d) { live { let x = 1; } } main(0);";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        assert!(
            find_first_live_timeout_ns(&program).is_none(),
            "plain `live` must not carry a timeout",
        );
    }

    #[test]
    fn live_within_tight_budget_exhausts_with_timeout_prefix() {
        // A permanently-failing body under a 1ms wall-clock cap:
        // the retry loop must stop on the timeout rather than at
        // MAX_RETRIES. We distinguish the two by the error prefix
        // — "timed out" vs "failed after 3 attempts".
        //
        // Forcing a detectable timeout requires at least one
        // measurable delay between attempts. A 2ms backoff per
        // retry puts elapsed >= 2ms on the first retry, well past
        // the 1ms cap.
        let src = "\
            fn always_fail() {\n\
                assert(false, \"forced\");\n\
                return 0;\n\
            }\n\
            fn main(int _d) {\n\
                live backoff(base_ms=2, factor=1, max_ms=2) within 1ms {\n\
                    let r = always_fail();\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp.eval(&program).unwrap_err();
        assert!(
            err.contains("Live block timed out"),
            "expected `timed out` prefix, got: {}",
            err
        );
    }

    #[test]
    fn live_within_slack_budget_succeeds() {
        // Same body shape but wider budget (1s) — the retry path
        // succeeds on the third try well inside the cap, so the
        // block returns normally.
        let src = "\
            static let fails_left = 2;\n\
            fn maybe_fail() {\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced\");\n\
                }\n\
                return 42;\n\
            }\n\
            fn main(int _d) {\n\
                live within 1s {\n\
                    let r = maybe_fail();\n\
                }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        // If this errored we'd have "timed out" or "failed after".
        // A clean Ok means the slack budget was enough.
        interp.eval(&program).expect("slack budget should succeed");
    }

    #[test]
    fn duration_literal_in_expression_position_is_rejected() {
        // Defensive: `Node::DurationLiteral` never appears outside
        // a live-within clause in well-formed source, but if one
        // does reach eval (e.g. a hand-rolled AST in a future
        // test), fail with the dedicated diagnostic rather than
        // silently coercing.
        use span::Span;
        let dl = Node::DurationLiteral { nanos: 1_000_000, span: Span::default() };
        let mut interp = Interpreter::new();
        let err = interp.eval(&dl).unwrap_err();
        assert!(
            err.contains("duration literals are only valid inside `live within"),
            "expected duration-literal-guard diagnostic, got: {}",
            err
        );
    }

    // --- RES-138: live_retries() builtin ---

    #[test]
    fn live_retries_outside_live_block_errors() {
        // Outside any live block — the thread-local stack is empty,
        // the builtin returns the dedicated diagnostic.
        let err = builtin_live_retries(&[]).unwrap_err();
        assert!(
            err.contains("called outside a live block"),
            "expected outside-block diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn live_retries_wrong_arity_errors() {
        let err = builtin_live_retries(&[Value::Int(1)]).unwrap_err();
        assert!(
            err.contains("expected 0 arguments"),
            "expected arity diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn live_retries_counts_up_across_failures() {
        // Force two failures, succeed on the third attempt. Collect
        // the `live_retries()` value on each attempt; must read
        // 0, 1, 2 in order.
        let src = "\
            static let fails_left = 2;\n\
            static let seen = [];\n\
            fn step() {\n\
                seen = push(seen, live_retries());\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced\");\n\
                }\n\
                return 42;\n\
            }\n\
            fn main(int _d) {\n\
                live { let r = step(); }\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        match interp.statics.borrow().get("seen").expect("static `seen`") {
            Value::Array(items) => {
                let ns: Vec<i64> = items
                    .iter()
                    .map(|v| match v {
                        Value::Int(n) => *n,
                        other => panic!("non-int in seen: {:?}", other),
                    })
                    .collect();
                assert_eq!(
                    ns,
                    vec![0, 1, 2],
                    "live_retries should count 0, 1, 2 across attempts"
                );
            }
            other => panic!("expected Array for seen, got {:?}", other),
        }
    }

    #[test]
    fn live_retries_after_block_exit_errors() {
        // Sanity check on the RAII guard: after a live block
        // completes, `live_retries()` must again be an error.
        // Run through the runtime-error formatter since the call is
        // at top level after the live block closes.
        let src = "\
            fn main(int _d) {\n\
                live { let x = 1; }\n\
                let r = live_retries();\n\
            }\n\
            main(0);\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        let err = interp
            .eval(&program)
            .expect_err("post-live-block call should fail");
        assert!(
            err.contains("called outside a live block"),
            "expected outside-block diagnostic, got: {}",
            err
        );
    }

    // --- RES-130: no implicit int ↔ float coercion ---

    /// Helper: assert that typechecking `src` fails with a message
    /// mentioning both `int and float` and the `to_float` /
    /// `to_int` hint — the coercion-policy diagnostic shape.
    fn assert_coercion_error(src: &str) {
        let (program, parse_errs) = parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {:?}", parse_errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("mixed int/float arith must fail typecheck");
        assert!(
            err.contains("int and float"),
            "expected coercion-policy diagnostic, got: {}",
            err
        );
        assert!(
            err.contains("to_float") || err.contains("to_int"),
            "diagnostic should mention the explicit conversion hint: {}",
            err
        );
    }

    #[test]
    fn no_coercion_plus()  { assert_coercion_error("let a = 1 + 2.0;"); }
    #[test]
    fn no_coercion_minus() { assert_coercion_error("let a = 1 - 2.0;"); }
    #[test]
    fn no_coercion_mul()   { assert_coercion_error("let a = 1 * 2.0;"); }
    #[test]
    fn no_coercion_div()   { assert_coercion_error("let a = 1 / 2.0;"); }
    #[test]
    fn no_coercion_mod()   { assert_coercion_error("let a = 1 % 2.0;"); }

    #[test]
    fn no_coercion_float_int_reversed() {
        // Symmetry: float-on-left / int-on-right also rejects.
        assert_coercion_error("let a = 1.0 + 2;");
    }

    #[test]
    fn to_float_then_arith_succeeds() {
        // Explicit conversion fixes the mismatch — same-type
        // arithmetic proceeds normally.
        let (program, errs) = parse("let a = to_float(1) + 2.0;");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        assert!(
            tc.check_program(&program).is_ok(),
            "typecheck of explicit to_float should succeed"
        );
    }

    #[test]
    fn to_int_nan_is_runtime_error() {
        // `to_int(NaN)` surfaces a clean runtime error rather than
        // silently producing `0` / `i64::MIN`. Driven through the
        // builtin directly because Resilient doesn't currently have
        // a surface-syntax way to produce a NaN (float `0.0 / 0.0`
        // is caught by the interpreter's divide-by-zero guard
        // BEFORE IEEE-754 semantics kick in).
        let err = builtin_to_int(&[Value::Float(f64::NAN)]).unwrap_err();
        assert!(
            err.contains("NaN"),
            "expected NaN diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn to_int_infinity_is_runtime_error() {
        let err = builtin_to_int(&[Value::Float(f64::INFINITY)]).unwrap_err();
        assert!(err.contains("positive infinity"), "got: {}", err);
        let err = builtin_to_int(&[Value::Float(f64::NEG_INFINITY)]).unwrap_err();
        assert!(err.contains("negative infinity"), "got: {}", err);
    }

    #[test]
    fn to_float_round_trip_preserves_int() {
        let (program, errs) = parse("let n = to_int(to_float(42));");
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut interp = Interpreter::new();
        interp.eval(&program).unwrap();
        assert!(matches!(interp.env.get("n"), Some(Value::Int(42))));
    }

    // --- RES-128: type alias declarations ---

    #[test]
    fn type_alias_accepts_structurally_compatible_value() {
        // `type M = int` + `fn inc(M x) -> M`: aliases are
        // structural, so `int` flows freely through `M`.
        let src = "\
            type M = int;\n\
            fn inc(M x) -> M { return x + 1; }\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        assert!(tc.check_program(&program).is_ok());
    }

    #[test]
    fn type_alias_rejects_wrong_value_type() {
        // `let m: M = "hi";` where `M = int` — still a type error,
        // with the alias expanded in the diagnostic message.
        let src = "\
            type M = int;\n\
            let m: M = \"hi\";\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("string cannot flow into int-via-alias");
        assert!(
            err.contains("let m: int") && err.contains("value has type string"),
            "expected alias-expanded diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn type_alias_cycle_is_diagnostic_not_panic() {
        // `A` → `B` → `A` cycle. Must surface as a clean
        // diagnostic — no stack overflow, no panic.
        let src = "\
            type A = B;\n\
            type B = A;\n\
            let a: A = 1;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("alias cycle must fail typecheck");
        assert!(
            err.contains("type alias cycle"),
            "expected cycle diagnostic, got: {}",
            err
        );
        // Chain is rendered for debuggability.
        assert!(
            err.contains("A") && err.contains("B"),
            "expected cycle chain to mention both aliases, got: {}",
            err
        );
    }

    #[test]
    fn type_alias_forward_reference_works() {
        // `fn foo(M x)` references `M` before `type M = int;` is
        // declared. The RES-128 hoisting pass in check_program_
        // with_source must register aliases BEFORE per-stmt walks,
        // same as the RES-061 contract-table pass.
        let src = "\
            fn inc(M x) -> M { return x + 1; }\n\
            type M = int;\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        assert!(
            tc.check_program_with_source(&program, "<test>").is_ok(),
            "forward ref should typecheck"
        );
    }

    // --- RES-126: nominal struct equivalence ---

    #[test]
    fn nominal_distinct_empty_braces() {
        // Two zero-field structs must still be distinct types —
        // assignment across them is a type error. This pins the
        // rule against accidental structural collapse in a later
        // refactor.
        let src = "\
            struct A { }\n\
            struct B { }\n\
            let a: A = new B { };\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("A and B must be nominally distinct even with zero fields");
        assert!(
            err.contains("let a: A") && err.contains("value has type B"),
            "expected nominal mismatch diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn nominal_distinct_same_shape() {
        // Two structs with identical `int val` fields still must
        // not unify. This is the RES-126 canonical case —
        // `Meters { val: 5 }` must not flow into a `Seconds`.
        let src = "\
            struct Meters { int val, }\n\
            struct Seconds { int val, }\n\
            let s: Seconds = new Meters { val: 5 };\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("Meters and Seconds must not unify despite same shape");
        assert!(
            err.contains("let s: Seconds") && err.contains("value has type Meters"),
            "expected nominal mismatch diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn nominal_same_name_accepts() {
        // Sanity: an assignment from the same named struct still
        // works — nominal distinctness is about different NAMES,
        // not about rejecting every struct-valued assignment.
        let src = "\
            struct Point { int x, int y, }\n\
            let p: Point = new Point { x: 1, y: 2 };\n\
        ";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let res = tc.check_program(&program);
        assert!(res.is_ok(), "same-named struct assignment should pass: {:?}", res);
    }

    #[test]
    fn fn_without_return_type_annotation_typechecks() {
        // Omitting `-> TYPE` is supported: the typechecker falls
        // through to the body's inferred type (`body_type` in
        // `check_node`'s Function arm).
        let src = "fn square(int x) { return x * x; }";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let res = tc.check_program(&program);
        assert!(res.is_ok(), "typecheck should succeed: {:?}", res);
    }

    #[test]
    fn fn_with_explicit_return_type_still_checks_against_body() {
        // Explicit annotation still overrides — declaring `-> bool`
        // against an `int` body is the canonical RES-053 mismatch
        // error, unchanged by RES-123.
        let src = "fn square(int x) -> bool { return x * x; }";
        let (program, errs) = parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        let mut tc = typechecker::TypeChecker::new();
        let err = tc
            .check_program(&program)
            .expect_err("bool-declared int body should fail typecheck");
        assert!(
            err.contains("return type mismatch"),
            "expected return-type mismatch diagnostic, got: {}",
            err
        );
    }

    #[test]
    fn fn_with_no_return_stmt_infers_void() {
        // Body with no `return` should infer `Type::Void`. We
        // verify by declaring an explicit `-> void` on ONE overload
        // and omitting the annotation on another — both must
        // typecheck against the same body.
        let src_explicit = "fn sink(int x) -> void { println(x); }";
        let (p1, e1) = parse(src_explicit);
        assert!(e1.is_empty(), "parse errors: {:?}", e1);
        let mut tc1 = typechecker::TypeChecker::new();
        assert!(
            tc1.check_program(&p1).is_ok(),
            "explicit `-> void` should typecheck"
        );

        let src_inferred = "fn sink(int x) { println(x); }";
        let (p2, e2) = parse(src_inferred);
        assert!(e2.is_empty(), "parse errors: {:?}", e2);
        let mut tc2 = typechecker::TypeChecker::new();
        assert!(
            tc2.check_program(&p2).is_ok(),
            "inferred-void body should typecheck"
        );
    }

    #[test]
    fn token_display_syntax_renders_source_form() {
        // Punctuation → backticked source form. Keywords → backticked
        // source form. Identifier payload appears inline.
        assert_eq!(Token::Semicolon.display_syntax(), "`;`");
        assert_eq!(Token::LeftBrace.display_syntax(), "`{`");
        assert_eq!(Token::Function.display_syntax(), "`fn`");
        assert_eq!(
            Token::Identifier("x".to_string()).display_syntax(),
            "identifier `x`"
        );
        assert_eq!(Token::Eof.display_syntax(), "end of input");
    }

    #[test]
    fn lexer_keeps_utf8_in_string_literals() {
        // RES-114: the policy ONLY tightens identifier scanning;
        // string bodies retain full UTF-8.
        let src = r#"let s = "Привет, мир";"#;
        let (_program, errs) = parse(src);
        assert!(
            errs.is_empty(),
            "UTF-8 inside a string literal must parse, got: {:?}",
            errs
        );
    }

    // ---------- RES-187: semantic tokens ----------

    /// The wire format is `[dLine, dStart, length, type, mods]*`.
    /// Encoding two tokens on the same line should emit a zero
    /// deltaLine and a column-difference deltaStart.
    #[test]
    fn encode_semantic_tokens_delta_encodes_same_line() {
        let tokens = vec![
            AbsSemToken { line: 0, col: 0, length: 3, ty: sem_tok::KEYWORD, modifiers: 0 },
            AbsSemToken { line: 0, col: 4, length: 3, ty: sem_tok::FUNCTION,
                          modifiers: sem_tok::MOD_DECLARATION },
        ];
        let wire = encode_semantic_tokens(&tokens);
        // First token: dLine=0, dStart=0, len=3, type=0, mods=0
        assert_eq!(&wire[0..5], &[0, 0, 3, sem_tok::KEYWORD, 0]);
        // Second: same line → dLine=0, dStart=4
        assert_eq!(&wire[5..10], &[0, 4, 3, sem_tok::FUNCTION, sem_tok::MOD_DECLARATION]);
    }

    /// Tokens on later lines encode an absolute deltaStart (the
    /// LSP spec resets `prev_col` to zero at every new line).
    #[test]
    fn encode_semantic_tokens_delta_encodes_across_lines() {
        let tokens = vec![
            AbsSemToken { line: 0, col: 0, length: 3, ty: sem_tok::KEYWORD, modifiers: 0 },
            AbsSemToken { line: 2, col: 4, length: 5, ty: sem_tok::VARIABLE, modifiers: 0 },
        ];
        let wire = encode_semantic_tokens(&tokens);
        // First: line 0 col 0.
        assert_eq!(&wire[0..5], &[0, 0, 3, sem_tok::KEYWORD, 0]);
        // Second: dLine=2 (0→2), dStart=4 (absolute — prev line
        // reset because dLine != 0).
        assert_eq!(&wire[5..10], &[2, 4, 5, sem_tok::VARIABLE, 0]);
    }

    /// `encode_semantic_tokens` must sort by (line, col) — the
    /// lex pass and the comment-scan pass each emit in their
    /// own order, and a post-merge sort is cheaper than threading
    /// insertion order through both.
    #[test]
    fn encode_semantic_tokens_sorts_by_position() {
        let tokens = vec![
            // Out-of-order: comment on line 2 first, then a
            // line-0 keyword.
            AbsSemToken { line: 2, col: 0, length: 4, ty: sem_tok::COMMENT, modifiers: 0 },
            AbsSemToken { line: 0, col: 0, length: 2, ty: sem_tok::KEYWORD, modifiers: 0 },
        ];
        let wire = encode_semantic_tokens(&tokens);
        // After sort: keyword first.
        assert_eq!(&wire[0..5], &[0, 0, 2, sem_tok::KEYWORD, 0]);
        assert_eq!(&wire[5..10], &[2, 0, 4, sem_tok::COMMENT, 0]);
    }

    #[test]
    fn encode_semantic_tokens_empty_returns_empty() {
        assert!(encode_semantic_tokens(&[]).is_empty());
    }

    /// `fn` followed by an identifier should tag the identifier
    /// as FUNCTION + DECLARATION. The default (no keyword
    /// context) should classify identifiers as plain VARIABLE.
    #[test]
    fn classify_lex_token_identifier_after_fn_is_function_declaration() {
        let src = "fn alpha() { return 0; }";
        let tokens = collect_semantic_tokens(src);
        // Expect tokens: fn (KEYWORD), alpha (FUNCTION+DECLARATION),
        // return (KEYWORD), 0 (NUMBER).
        let by_kind: Vec<(u32, u32)> = tokens.iter()
            .map(|t| (t.ty, t.modifiers))
            .collect();
        assert!(
            by_kind.contains(&(sem_tok::FUNCTION, sem_tok::MOD_DECLARATION)),
            "expected FUNCTION+DECLARATION for `alpha`, got: {:?}",
            by_kind
        );
    }

    /// `struct` and `type` should both tag their identifier
    /// as TYPE + DECLARATION; `new` should tag its following
    /// identifier as plain TYPE (no DECLARATION — it's a use,
    /// not a define).
    #[test]
    fn classify_lex_token_struct_type_new_all_tag_type() {
        let src = "struct Point { int x }\ntype Meters = int;\nfn f() { let p = new Point(); return 0; }";
        let tokens = collect_semantic_tokens(src);
        let by_kind: Vec<(u32, u32)> = tokens.iter()
            .map(|t| (t.ty, t.modifiers))
            .collect();
        // `Point` appears twice — once as a struct declaration,
        // once via `new`. At least one of each variant should
        // show up.
        assert!(
            by_kind.contains(&(sem_tok::TYPE, sem_tok::MOD_DECLARATION)),
            "expected TYPE+DECLARATION (Point or Meters), got: {:?}",
            by_kind
        );
        // `new Point()` → TYPE without DECLARATION.
        assert!(
            by_kind.contains(&(sem_tok::TYPE, 0)),
            "expected TYPE (bare) after `new`, got: {:?}",
            by_kind
        );
    }

    /// A standalone number literal should be tagged NUMBER, a
    /// string literal STRING. Comments flow through a separate
    /// scan.
    #[test]
    fn collect_semantic_tokens_tags_literals_and_comments() {
        let src = "// hi\nlet s = \"abc\";\nlet n = 42;";
        let tokens = collect_semantic_tokens(src);
        let tys: Vec<u32> = tokens.iter().map(|t| t.ty).collect();
        assert!(tys.contains(&sem_tok::COMMENT), "missing COMMENT: {:?}", tys);
        assert!(tys.contains(&sem_tok::STRING), "missing STRING: {:?}", tys);
        assert!(tys.contains(&sem_tok::NUMBER), "missing NUMBER: {:?}", tys);
    }

    /// Operators (`+`, `==`, `=`, …) should tag as OPERATOR.
    #[test]
    fn collect_semantic_tokens_tags_operators() {
        let src = "let x = 1 + 2; let y = x == 3;";
        let tokens = collect_semantic_tokens(src);
        let op_count = tokens.iter().filter(|t| t.ty == sem_tok::OPERATOR).count();
        assert!(
            op_count >= 3,
            "expected at least 3 OPERATOR tokens (= + ==), got {}",
            op_count
        );
    }

    /// AC of RES-187 calls for "a small program with each token
    /// type represented". This pins exactly that: every indexed
    /// token type should appear in the output of a modest test
    /// program that spans keyword, fn decl, variable use,
    /// parameter name, type name, string, number, comment, and
    /// operator.
    #[test]
    fn collect_semantic_tokens_covers_all_token_types() {
        // Program carefully constructed to exercise every type:
        // - `fn`             → KEYWORD
        // - `greet`          → FUNCTION + DECLARATION
        // - `name`           → VARIABLE or PARAMETER (parameter
        //                      detection is best-effort; we only
        //                      require VARIABLE to appear)
        // - `string`         → VARIABLE (bare identifier used as
        //                      a type annotation; for coverage we
        //                      only require TYPE to show up
        //                      elsewhere)
        // - `Point`          → TYPE + DECLARATION (struct decl)
        // - `"hi"`           → STRING
        // - `0`, `1`         → NUMBER
        // - `// …`           → COMMENT
        // - `+`, `=`         → OPERATOR
        let src = "\
            // header comment\n\
            struct Point { int x }\n\
            fn greet(int name) {\n\
                let s = \"hi\";\n\
                let n = name + 1;\n\
                return 0;\n\
            }\n\
        ";
        let tokens = collect_semantic_tokens(src);
        let types_seen: std::collections::HashSet<u32> =
            tokens.iter().map(|t| t.ty).collect();
        for want in [
            sem_tok::KEYWORD, sem_tok::FUNCTION, sem_tok::VARIABLE,
            sem_tok::TYPE, sem_tok::STRING, sem_tok::NUMBER,
            sem_tok::COMMENT, sem_tok::OPERATOR,
        ] {
            assert!(
                types_seen.contains(&want),
                "expected token type {} in output, got types {:?}",
                want, types_seen
            );
        }
        // At least one DECLARATION modifier should appear (on
        // `greet` and `Point`).
        let any_decl = tokens.iter()
            .any(|t| t.modifiers & sem_tok::MOD_DECLARATION != 0);
        assert!(any_decl, "expected a DECLARATION modifier somewhere");
    }

    /// Round-trip: compute_semantic_tokens should produce a Vec
    /// whose length is a multiple of 5, and whose first triple
    /// of integers (dLine, dStart, length) points at a plausible
    /// source location.
    #[test]
    fn compute_semantic_tokens_returns_wire_format() {
        let src = "fn f() { return 0; }";
        let wire = compute_semantic_tokens(src);
        assert!(!wire.is_empty(), "expected tokens for non-empty program");
        assert_eq!(
            wire.len() % 5, 0,
            "wire format must be 5-tuples, got len {}",
            wire.len()
        );
        // First token starts at column 0 of line 0.
        assert_eq!(wire[0], 0, "first dLine should be 0");
        assert_eq!(wire[1], 0, "first dStart should be 0");
    }
}