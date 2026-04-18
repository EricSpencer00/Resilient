// RES-196: bootstrap experiment — a lexer for a restricted subset
// of Resilient, written in Resilient itself. Runs against the Rust
// interpreter; output is compared to a reference snapshot.
//
// Scope (what this prototype recognizes):
//   - identifiers: `[A-Za-z_][A-Za-z0-9_]*`
//   - integer literals: `[0-9]+`
//   - string literals: `"..."` with no escape processing
//   - keywords: `fn`, `let`, `return`, `if`, `else`, `while`,
//     `true`, `false`
//   - single-char punctuation: `( ) { } [ ] ; , : .`
//   - operators: `+ - * / = == != < > <= >= && || !`
//   - line comments: `//` to end-of-line (produces no token)
//   - whitespace: skipped
//
// Anything else (multiline strings, block comments, struct/match,
// `live`, contract keywords, float/bytes literals) is either
// skipped as whitespace or emitted as an `UNKNOWN` token. A
// production self-hosted lexer would cover those — this is a
// prototype that proves the language can express the core
// scanning loop.
//
// Output format: one token per line, `KIND LEXEME LINE COL`.
//   - LINE and COL are 1-indexed and point at the first
//     character of the token (matching the Rust lexer's span
//     convention).
//   - KIND is one of: IDENT, INT, STRING, KW, PUNCT, OP,
//     UNKNOWN, EOF.
//
// The driver reads the input path from `SELF_HOST_INPUT` below
// at the top of main. The script in `self-host/run.sh` passes
// the expected hello.rs path.
//
// Parser workarounds: the current Resilient parser chokes on
// parenthesized expressions of the form `(a - b)` and
// `(a <= b)`, so every arithmetic / comparison expression in
// this file is either naked or hoisted into a named `let`.

// --- Character classification helpers ---

fn is_digit(string c) {
    return c >= "0" && c <= "9";
}

fn is_alpha(string c) {
    // ASCII letters + underscore. Resilient identifier lexer is
    // ASCII-only per RES-114, so we don't need broader Unicode.
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
    let g1 = c == "(" || c == ")" || c == "{" || c == "}";
    let g2 = c == "[" || c == "]" || c == ";" || c == ",";
    let g3 = c == ":" || c == ".";
    return g1 || g2 || g3;
}

// Is `w` a Resilient keyword (in our restricted subset)?
fn is_keyword(string w) {
    let g1 = w == "fn" || w == "let" || w == "return";
    let g2 = w == "if" || w == "else" || w == "while";
    let g3 = w == "true" || w == "false";
    return g1 || g2 || g3;
}

fn is_single_op(string c) {
    let g1 = c == "+" || c == "-" || c == "*" || c == "/";
    let g2 = c == "=" || c == "<" || c == ">" || c == "!";
    return g1 || g2;
}

// --- Token emission ---

// Tiny int → decimal string. Resilient's `format` builtin takes
// an `(fmt, array)` signature — wrapping single ints in an array
// per call is noisier than this helper.
fn int_to_str(int n) {
    if n == 0 {
        return "0";
    }
    // Buffer the digits in reverse, then flip.
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
// so the main loop can update cursor state without tuples.

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

fn scan_integer(array chars, int i, int line, int col) {
    let start = i;
    while i < len(chars) && is_digit(chars[i]) {
        i = i + 1;
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

fn scan_string(array chars, int i, int line, int col) {
    // Enter on the opening `"`. Consume until the matching `"`;
    // no escape processing in this prototype.
    i = i + 1; // skip opening quote
    let line_after = line;
    let col_after = col + 1;
    let lex = "\"";
    while i < len(chars) && chars[i] != "\"" {
        lex = lex + chars[i];
        if chars[i] == "\n" {
            line_after = line_after + 1;
            col_after = 1;
        } else {
            col_after = col_after + 1;
        }
        i = i + 1;
    }
    if i < len(chars) {
        lex = lex + "\"";
        i = i + 1;
        col_after = col_after + 1;
    }
    let tok = format_token("STRING", lex, line, col);
    return new ScanStep { next_i: i, next_line: line_after, next_col: col_after, emit: tok };
}

// Try a two-char operator starting at `i`. Returns empty string
// when no two-char op matches.
fn try_two_char_op(array chars, int i) {
    let nxt = i + 1;
    if nxt >= len(chars) {
        return "";
    }
    let pair = chars[i] + chars[nxt];
    let g = pair == "==" || pair == "!=" || pair == "<=" || pair == ">=";
    let h = pair == "&&" || pair == "||";
    if g || h {
        return pair;
    }
    return "";
}

// --- Main scanner loop ---

fn main(int _d) {
    let input_path = "resilient/examples/hello.rs";
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
            // Line comment `//...<newline>`.
            let nxt = i + 1;
            let is_comment = false;
            if c == "/" && nxt < n {
                if chars[nxt] == "/" {
                    is_comment = true;
                    while i < n && chars[i] != "\n" {
                        i = i + 1;
                        col = col + 1;
                    }
                    // Leave the newline for the outer loop.
                }
            }

            if is_comment == false {
                if is_alpha(c) {
                    let step = scan_identifier(chars, i, line, col);
                    tokens = push(tokens, step.emit);
                    i = step.next_i;
                    line = step.next_line;
                    col = step.next_col;
                } else {
                    if is_digit(c) {
                        let step = scan_integer(chars, i, line, col);
                        tokens = push(tokens, step.emit);
                        i = step.next_i;
                        line = step.next_line;
                        col = step.next_col;
                    } else {
                        if c == "\"" {
                            let step = scan_string(chars, i, line, col);
                            tokens = push(tokens, step.emit);
                            i = step.next_i;
                            line = step.next_line;
                            col = step.next_col;
                        } else {
                            let two = try_two_char_op(chars, i);
                            if two != "" {
                                tokens = push(tokens, format_token("OP", two, line, col));
                                i = i + 2;
                                col = col + 2;
                            } else {
                                if is_punct(c) {
                                    tokens = push(tokens, format_token("PUNCT", c, line, col));
                                    i = i + 1;
                                    col = col + 1;
                                } else {
                                    if is_single_op(c) {
                                        tokens = push(tokens, format_token("OP", c, line, col));
                                        i = i + 1;
                                        col = col + 1;
                                    } else {
                                        tokens = push(tokens, format_token("UNKNOWN", c, line, col));
                                        i = i + 1;
                                        col = col + 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // EOF marker so the snapshot is explicit about where the
    // file ended.
    tokens = push(tokens, format_token("EOF", "", line, col));

    let k = 0;
    while k < len(tokens) {
        println(tokens[k]);
        k = k + 1;
    }

    return 0;
}
main(0);
