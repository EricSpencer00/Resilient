// RES-323: Resilient lexer in Resilient itself (self-hosting step 1).
//
// This is the production-step-up from the RES-196 prototype in
// `self-host/lex.rs`. It keeps that prototype's scanning skeleton
// (alpha/digit predicates, per-kind scanners, ScanStep struct) and
// extends it with:
//
//   - Larger keyword set covering Resilient's verification surface
//     (requires, ensures, invariant, assume, assert, recovers_to,
//     forall, exists, struct, enum, match, impl, trait, actor, ...).
//   - String escape sequences `\n \t \r \\ \" \0`.
//   - Block comments `/* ... */` (non-nesting).
//   - Hex (`0x...`) and binary (`0b...`) integer literals.
//   - Three-character operators `..= ... <<= >>=`.
//   - Two-character operators `-> => :: == != <= >= && || << >> += -= *= /= %= ..` .
//   - `%` `^` `&` `|` `?` `~` `@` as single-char operators.
//
// Output format (one line per token, identical to the RES-196
// prototype so existing harnesses keep working):
//
//     KIND LEXEME LINE COL
//
// where KIND ∈ {IDENT, INT, FLOAT, STRING, KW, PUNCT, OP, UNKNOWN, EOF}
// and LINE/COL are 1-indexed and point at the first character of
// the token. The trailing EOF token has empty LEXEME and points at
// the position one past the final character.
//
// Driver:
//   The input file path comes from the `SELF_HOST_INPUT` environment
//   variable. Falling back to `resilient/examples/hello.rz` when
//   unset keeps the snapshot harness compatible with the RES-196
//   driver shape (which had the path baked in). The lexer prints
//   tokens to stdout; `self-host/lexer_check.sh` is the driver that
//   compares stdout against `self-host/expected/*.tokens.txt`.
//
// Parser workarounds (carried over from RES-196): the current
// Resilient parser still chokes on parenthesized expressions in some
// positions, so every arithmetic / comparison expression here is
// either naked or hoisted into a named `let`.

// --- Character classification helpers ---

fn is_digit(string c) {
    return c >= "0" && c <= "9";
}

fn is_hex_digit(string c) {
    let dec = c >= "0" && c <= "9";
    let lo = c >= "a" && c <= "f";
    let hi = c >= "A" && c <= "F";
    return dec || lo || hi;
}

fn is_bin_digit(string c) {
    return c == "0" || c == "1";
}

fn is_alpha(string c) {
    let lo = c >= "a" && c <= "z";
    let hi = c >= "A" && c <= "Z";
    return lo || hi || c == "_";
}

fn is_alnum(string c) {
    return is_alpha(c) || is_digit(c);
}

fn is_whitespace(string c) {
    return c == " " || c == "\t" || c == "\n" || c == "\r";
}

fn is_punct(string c) {
    // `"\{"` produces a literal `{` — bare `"{"` would be parsed as
    // the start of a string-interpolation expression (RES-221).
    let g1 = c == "(" || c == ")" || c == "\{" || c == "}";
    let g2 = c == "[" || c == "]" || c == ";" || c == ",";
    let g3 = c == ":" || c == ".";
    return g1 || g2 || g3;
}

// Single-character operator catalog. The lexer tries multi-char
// matches first and only falls back to this when no longer match
// applies — so e.g. `=` reaches here only when the next character
// isn't `=` or `>`.
fn is_single_op(string c) {
    let g1 = c == "+" || c == "-" || c == "*" || c == "/";
    let g2 = c == "=" || c == "<" || c == ">" || c == "!";
    let g3 = c == "%" || c == "^" || c == "&" || c == "|";
    let g4 = c == "?" || c == "~" || c == "@";
    return g1 || g2 || g3 || g4;
}

// Keyword recognizer. The set deliberately mirrors what the Rust
// `lexer_logos.rs` <EXTENSION_TOKENS> block recognizes today —
// adding a new arm to either side without the other will surface as
// an UNKNOWN/IDENT diff in the cross-check harness.
//
// Grouped 3-up to keep the Resilient parser happy under the current
// `(a || b || c)` parenthesization rules.
fn is_keyword(string w) {
    let g1 = w == "fn" || w == "let" || w == "return";
    let g2 = w == "if" || w == "else" || w == "while";
    let g3 = w == "for" || w == "in" || w == "loop";
    let g4 = w == "true" || w == "false" || w == "null";
    let g5 = w == "struct" || w == "enum" || w == "impl";
    let g6 = w == "trait" || w == "type" || w == "match";
    let g7 = w == "use" || w == "mod" || w == "pub";
    let g8 = w == "const" || w == "static" || w == "mut";
    let g9 = w == "ref" || w == "as" || w == "new";
    let g10 = w == "break" || w == "continue" || w == "where";
    let g11 = w == "requires" || w == "ensures" || w == "invariant";
    let g12 = w == "assume" || w == "assert" || w == "ghost";
    let g13 = w == "axiom" || w == "theorem" || w == "lemma";
    let g14 = w == "recovers_to" || w == "forall" || w == "exists";
    let g15 = w == "actor" || w == "receive" || w == "send";
    let g16 = w == "spawn" || w == "concurrent_ensures" || w == "always";
    let g17 = w == "eventually" || w == "try" || w == "catch";
    let g18 = w == "throw" || w == "live" || w == "result";
    let h1 = g1 || g2 || g3;
    let h2 = g4 || g5 || g6;
    let h3 = g7 || g8 || g9;
    let h4 = g10 || g11 || g12;
    let h5 = g13 || g14 || g15;
    let h6 = g16 || g17 || g18;
    return h1 || h2 || h3 || h4 || h5 || h6;
}

// --- Token emission ---

// Tiny int → decimal string. `format` is overkill for single ints
// and routes through Resilient's printer infrastructure; this avoids
// that dependency so the harness stays focused on the lexer.
fn int_to_str(int n) {
    if n == 0 {
        return "0";
    }
    let digits = [];
    let m = n;
    let neg = false;
    if m < 0 {
        neg = true;
        m = 0 - m;
    }
    while m > 0 {
        let d = m % 10;
        digits = push(digits, d);
        m = m / 10;
    }
    let s = "";
    let i = len(digits) - 1;
    while i >= 0 {
        let ch = "0";
        let d = digits[i];
        if d == 1 { ch = "1"; }
        if d == 2 { ch = "2"; }
        if d == 3 { ch = "3"; }
        if d == 4 { ch = "4"; }
        if d == 5 { ch = "5"; }
        if d == 6 { ch = "6"; }
        if d == 7 { ch = "7"; }
        if d == 8 { ch = "8"; }
        if d == 9 { ch = "9"; }
        s = s + ch;
        i = i - 1;
    }
    if neg {
        s = "-" + s;
    }
    return s;
}

fn format_token(string kind, string lexeme, int line, int col) {
    return kind + " " + lexeme + " " + int_to_str(line) + " " + int_to_str(col);
}

// --- Per-kind scanners ---
//
// Each returns `new ScanStep { next_i, next_line, next_col, emit }`
// so the main loop updates cursor state without tuples (Resilient's
// tuple support is still maturing — RES-127).

struct ScanStep {
    int next_i,
    int next_line,
    int next_col,
    string emit
}

fn scan_identifier(array chars, int i, int line, int col) {
    let start = i;
    while i < len(chars) && is_alnum(chars[i]) {
        i = i + 1;
    }
    let lex = "";
    let j = start;
    while j < i {
        lex = lex + chars[j];
        j = j + 1;
    }
    let kind = "IDENT";
    if is_keyword(lex) {
        kind = "KW";
    }
    let span = i - start;
    let tok = format_token(kind, lex, line, col);
    return new ScanStep { next_i: i, next_line: line, next_col: col + span, emit: tok };
}

fn scan_decimal(array chars, int i, int line, int col) {
    let start = i;
    while i < len(chars) && is_digit(chars[i]) {
        i = i + 1;
    }
    // Optional fractional part — `.` followed by at least one digit.
    // We do NOT consume a trailing `.` with no fractional digits;
    // that's reserved for range operators like `0..10`.
    let is_float = false;
    let nxt = i + 1;
    if i < len(chars) && chars[i] == "." {
        if nxt < len(chars) && is_digit(chars[nxt]) {
            is_float = true;
            i = i + 1;
            while i < len(chars) && is_digit(chars[i]) {
                i = i + 1;
            }
        }
    }
    let lex = "";
    let j = start;
    while j < i {
        lex = lex + chars[j];
        j = j + 1;
    }
    let span = i - start;
    let kind = "INT";
    if is_float {
        kind = "FLOAT";
    }
    let tok = format_token(kind, lex, line, col);
    return new ScanStep { next_i: i, next_line: line, next_col: col + span, emit: tok };
}

// `0x...` or `0b...` — caller has confirmed the leading `0` and we
// step into the prefix character here.
fn scan_radix(array chars, int i, int line, int col, string prefix) {
    let start = i;
    // skip "0" + prefix character
    i = i + 2;
    if prefix == "x" {
        while i < len(chars) && is_hex_digit(chars[i]) {
            i = i + 1;
        }
    } else {
        while i < len(chars) && is_bin_digit(chars[i]) {
            i = i + 1;
        }
    }
    let lex = "";
    let j = start;
    while j < i {
        lex = lex + chars[j];
        j = j + 1;
    }
    let span = i - start;
    let tok = format_token("INT", lex, line, col);
    return new ScanStep { next_i: i, next_line: line, next_col: col + span, emit: tok };
}

// String scanner with escape support. The lexeme retained in the
// output keeps the surrounding quotes and the literal escape
// sequences (e.g. `"a\nb"` stays `"a\nb"`). We emit the escape
// sequence verbatim — matching how `lexer_logos.rs` retains the raw
// span. Decoding to the runtime value is the parser's job.
fn scan_string(array chars, int i, int line, int col) {
    i = i + 1; // skip opening quote
    let line_after = line;
    let col_after = col + 1;
    let lex = "\"";
    let closed = false;
    while i < len(chars) && closed == false {
        let ch = chars[i];
        if ch == "\"" {
            closed = true;
            lex = lex + "\"";
            i = i + 1;
            col_after = col_after + 1;
        } else {
            if ch == "\\" && i + 1 < len(chars) {
                // Keep the backslash + escape character verbatim.
                let esc = chars[i + 1];
                lex = lex + ch;
                lex = lex + esc;
                i = i + 2;
                col_after = col_after + 2;
            } else {
                lex = lex + ch;
                if ch == "\n" {
                    line_after = line_after + 1;
                    col_after = 1;
                } else {
                    col_after = col_after + 1;
                }
                i = i + 1;
            }
        }
    }
    let kind = "STRING";
    if closed == false {
        kind = "UNKNOWN";
    }
    let tok = format_token(kind, lex, line, col);
    return new ScanStep { next_i: i, next_line: line_after, next_col: col_after, emit: tok };
}

// Three-character operator probe. Returns the matched operator or
// "" when no three-char op fits. `..=` and `<<=`/`>>=` are the
// current set; any future addition lands here.
fn try_three_char_op(array chars, int i) {
    let n = len(chars);
    let two_left = i + 2;
    if two_left >= n {
        return "";
    }
    let a = chars[i];
    let b = chars[i + 1];
    let c = chars[i + 2];
    let trip = a + b + c;
    let m1 = trip == "..=" || trip == "..<";
    let m2 = trip == "<<=" || trip == ">>=";
    let m3 = trip == "===" || trip == "!==";
    if m1 || m2 || m3 {
        return trip;
    }
    return "";
}

// Two-character operator probe. Order matters only when a prefix is
// shared with a three-char form; the caller checks three-char first.
fn try_two_char_op(array chars, int i) {
    let nxt = i + 1;
    if nxt >= len(chars) {
        return "";
    }
    let pair = chars[i] + chars[nxt];
    let g1 = pair == "==" || pair == "!=" || pair == "<=" || pair == ">=";
    let g2 = pair == "&&" || pair == "||" || pair == "<<" || pair == ">>";
    let g3 = pair == "->" || pair == "=>" || pair == "::" || pair == "..";
    let g4 = pair == "+=" || pair == "-=" || pair == "*=" || pair == "/=";
    let g5 = pair == "%=" || pair == "&=" || pair == "|=" || pair == "^=";
    if g1 || g2 || g3 || g4 || g5 {
        return pair;
    }
    return "";
}

// --- Driver ---

fn resolve_input_path() {
    // `env()` returns a Resilient `Result` — `is_ok` / `unwrap` are
    // the right accessors. The Result value is NOT a struct with
    // `.ok`/`.payload` fields at runtime even though the Rust-side
    // representation looks like one.
    let r = env("SELF_HOST_INPUT");
    if is_ok(r) {
        return unwrap(r);
    }
    // Fallback keeps the harness usable when the env var isn't set
    // (e.g. running `lexer.res` directly during local development).
    return "resilient/examples/hello.rz";
}

fn main(int _d) {
    let input_path = resolve_input_path();
    let src = file_read(input_path);
    let chars = split(src, "");
    let n = len(chars);

    let tokens = [];
    let i = 0;
    let line = 1;
    let col = 1;

    while i < n {
        let c = chars[i];

        if is_whitespace(c) {
            if c == "\n" {
                line = line + 1;
                col = 1;
            } else {
                col = col + 1;
            }
            i = i + 1;
        } else {
            let consumed = false;
            let nxt = i + 1;

            // Line comment `// ... \n`
            if c == "/" && nxt < n && consumed == false {
                if chars[nxt] == "/" {
                    consumed = true;
                    while i < n && chars[i] != "\n" {
                        i = i + 1;
                        col = col + 1;
                    }
                }
            }

            // Block comment `/* ... */` (non-nesting). Tracks line/col
            // through the body so the next token's position is correct.
            if c == "/" && nxt < n && consumed == false {
                if chars[nxt] == "*" {
                    consumed = true;
                    i = i + 2;
                    col = col + 2;
                    let closed = false;
                    while i < n && closed == false {
                        let inner = chars[i];
                        let after = i + 1;
                        let is_close = false;
                        if inner == "*" && after < n {
                            if chars[after] == "/" {
                                is_close = true;
                            }
                        }
                        if is_close {
                            closed = true;
                            i = i + 2;
                            col = col + 2;
                        } else {
                            if inner == "\n" {
                                line = line + 1;
                                col = 1;
                            } else {
                                col = col + 1;
                            }
                            i = i + 1;
                        }
                    }
                }
            }

            if consumed == false {
                if is_alpha(c) {
                    let step = scan_identifier(chars, i, line, col);
                    tokens = push(tokens, step.emit);
                    i = step.next_i;
                    line = step.next_line;
                    col = step.next_col;
                    consumed = true;
                }
            }

            // 0x... or 0b... before falling into the decimal scanner.
            if consumed == false {
                if c == "0" && nxt < n {
                    let p = chars[nxt];
                    if p == "x" || p == "X" {
                        let step = scan_radix(chars, i, line, col, "x");
                        tokens = push(tokens, step.emit);
                        i = step.next_i;
                        line = step.next_line;
                        col = step.next_col;
                        consumed = true;
                    } else {
                        if p == "b" || p == "B" {
                            let step = scan_radix(chars, i, line, col, "b");
                            tokens = push(tokens, step.emit);
                            i = step.next_i;
                            line = step.next_line;
                            col = step.next_col;
                            consumed = true;
                        }
                    }
                }
            }

            if consumed == false {
                if is_digit(c) {
                    let step = scan_decimal(chars, i, line, col);
                    tokens = push(tokens, step.emit);
                    i = step.next_i;
                    line = step.next_line;
                    col = step.next_col;
                    consumed = true;
                }
            }

            if consumed == false {
                if c == "\"" {
                    let step = scan_string(chars, i, line, col);
                    tokens = push(tokens, step.emit);
                    i = step.next_i;
                    line = step.next_line;
                    col = step.next_col;
                    consumed = true;
                }
            }

            if consumed == false {
                let three = try_three_char_op(chars, i);
                if three != "" {
                    tokens = push(tokens, format_token("OP", three, line, col));
                    i = i + 3;
                    col = col + 3;
                    consumed = true;
                }
            }

            if consumed == false {
                let two = try_two_char_op(chars, i);
                if two != "" {
                    tokens = push(tokens, format_token("OP", two, line, col));
                    i = i + 2;
                    col = col + 2;
                    consumed = true;
                }
            }

            if consumed == false {
                if is_punct(c) {
                    tokens = push(tokens, format_token("PUNCT", c, line, col));
                    i = i + 1;
                    col = col + 1;
                    consumed = true;
                }
            }

            if consumed == false {
                if is_single_op(c) {
                    tokens = push(tokens, format_token("OP", c, line, col));
                    i = i + 1;
                    col = col + 1;
                    consumed = true;
                }
            }

            if consumed == false {
                tokens = push(tokens, format_token("UNKNOWN", c, line, col));
                i = i + 1;
                col = col + 1;
            }
        }
    }

    tokens = push(tokens, format_token("EOF", "", line, col));

    let k = 0;
    while k < len(tokens) {
        println(tokens[k]);
        k = k + 1;
    }

    return 0;
}
main(0);
