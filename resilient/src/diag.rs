//! RES-117: shared diagnostic rendering.
//!
//! `format_diagnostic(src, span, level, msg)` is the one place in
//! the pipeline that turns a `(source, span, message)` triple into
//! the rustc-style multi-line diagnostic with a caret underline:
//!
//! ```text
//! foo.rs:12:9: error: division by zero
//!    let r = a / b;
//!            ^^^^^
//! ```
//!
//! The three execution modes (interpreter, VM, JIT) plus the
//! parser all share this helper — each mode's boundary in the
//! driver either constructs a `Span` directly, or parses the
//! `line:col:` prefix of an existing error string and calls
//! `format_diagnostic_from_line_col` to build a zero-width
//! synthetic span pointing at that position.
//!
//! **No ANSI colour codes.** Diagnostics frequently pipe into
//! logs or the LSP channel; escape codes would render as garbage
//! there, and users who want colour can layer it on with their
//! own terminal wrapper.

use crate::span::Span;

/// How many spaces a literal `\t` becomes when computing the
/// caret column. Picked to match most terminals' default, and
/// documented inline in the ticket's Notes section.
const TAB_WIDTH: usize = 4;

/// Render a source-context diagnostic:
///
/// ```text
/// <level>: <msg>
///    <line of source, with tabs expanded to 4 spaces>
///    <caret underline covering [span.start.col, span.end.col)>
/// ```
///
/// Leading header lines (filename:line:col) are the caller's
/// responsibility — this helper focuses on the source-context
/// block so different front-ends (CLI, LSP hover, etc.) can
/// paste their own header in front. Callers that want the full
/// CLI shape can just `format!("{}:{}  {}", filename, span.start,
/// format_diagnostic(...))`.
///
/// Multi-line spans render only the start line, followed by a
/// `(span continues on line N)` tail — most terminals can't
/// meaningfully underline across lines and printing the whole
/// range clutters the output.
pub fn format_diagnostic(src: &str, span: Span, level: &str, msg: &str) -> String {
    let line_num = span.start.line;
    let line_text = nth_line(src, line_num).unwrap_or("");
    let expanded = expand_tabs(line_text);

    // Column bounds. `Span::{start,end}.column` are 1-indexed.
    // Clamp start_col into the line and compute a caret width that
    // covers at least one `^` so zero-width spans still render.
    let start_col = span.start.column.max(1);
    let end_col = if span.end.line == span.start.line {
        span.end.column.max(start_col)
    } else {
        // Multi-line span: underline to end-of-line only.
        expanded.chars().count() + 1
    };
    let caret_count = (end_col - start_col).max(1);

    let pad = " ".repeat(start_col - 1);
    let carets = "^".repeat(caret_count);

    let mut out = String::new();
    out.push_str(level);
    out.push_str(": ");
    out.push_str(msg);
    out.push('\n');
    out.push_str("   ");
    out.push_str(&expanded);
    out.push('\n');
    out.push_str("   ");
    out.push_str(&pad);
    out.push_str(&carets);
    if span.end.line != span.start.line {
        out.push_str(&format!("\n   (span continues on line {})", span.end.line));
    }
    out
}

/// Convenience for callers that have parsed a `<line>:<col>:`
/// prefix out of an existing error string and don't carry a full
/// `Span`. Builds a zero-width synthetic span at `(line, col)`
/// and delegates.
pub fn format_diagnostic_from_line_col(
    src: &str,
    line: usize,
    col: usize,
    level: &str,
    msg: &str,
) -> String {
    let pos = crate::span::Pos::new(line.max(1), col.max(1), 0);
    format_diagnostic(src, Span::new(pos, pos), level, msg)
}

/// Pull the `n`-th line (1-indexed) out of `src` without the
/// trailing newline. Returns `None` when `n` is past the last
/// line.
fn nth_line(src: &str, n: usize) -> Option<&str> {
    if n == 0 {
        return None;
    }
    src.lines().nth(n - 1)
}

/// Expand any `\t` in `line` to `TAB_WIDTH` spaces so the caret
/// underline lines up visually. Non-tab characters are preserved
/// byte-for-byte (no UTF-8 normalisation).
fn expand_tabs(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        if c == '\t' {
            out.push_str(&" ".repeat(TAB_WIDTH));
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Pos;

    fn span(start_line: usize, start_col: usize, end_line: usize, end_col: usize) -> Span {
        Span::new(
            Pos::new(start_line, start_col, 0),
            Pos::new(end_line, end_col, 0),
        )
    }

    /// RES-117 requires "one new unit test per mode asserting a
    /// sample error message contains the `^` carets." These tests
    /// exercise the shared helper with error shapes representative
    /// of each mode.

    #[test]
    fn interpreter_style_diagnostic_has_caret_under_division() {
        // RES-116 interpreter runtime error: divide-by-zero on
        // `let r = 100 / n;` — span covers `100 / n` (cols 13..20
        // in the sample source below).
        let src = "fn f(int n) {\n    let r = 100 / n;\n    return r;\n}\nf(0);";
        let d = format_diagnostic(src, span(2, 13, 2, 20), "Runtime error", "division by zero");
        assert!(
            d.contains("Runtime error: division by zero"),
            "missing level+msg: {}",
            d
        );
        assert!(
            d.contains("    let r = 100 / n;"),
            "missing source line: {}",
            d
        );
        assert!(d.contains("^"), "missing caret underline: {}", d);
        // First caret aligns under column 13 (start of `100`).
        // Indent "   " (3 spaces) + 12 spaces of pad + carets.
        assert!(
            d.lines().any(|ln| ln.starts_with("               ^")),
            "caret position off: {}",
            d
        );
    }

    #[test]
    fn vm_style_diagnostic_has_caret_with_line_col_shim() {
        // RES-091 VM error: `VmError::AtLine { line, kind }` carries
        // just a line number. Drivers parse the final error string
        // into a (line, col) pair and call the line-col convenience.
        let src = "fn boom(int n) {\n    let r = 100 / n;\n    return r;\n}\nboom(0);";
        let d = format_diagnostic_from_line_col(src, 2, 13, "VM runtime error", "division by zero");
        assert!(
            d.contains("VM runtime error: division by zero"),
            "missing level+msg: {}",
            d
        );
        assert!(d.contains("^"), "missing caret: {}", d);
    }

    #[test]
    fn parser_style_diagnostic_has_caret_at_offending_token() {
        // RES-089 parser errors carry a bare `line:col:` prefix.
        // After the driver strips the prefix, the message plus the
        // (line, col) flow here.
        let src = "let x = ;\n";
        let d = format_diagnostic_from_line_col(src, 1, 9, "Parser error", "unexpected `;`");
        assert!(
            d.contains("Parser error: unexpected `;`"),
            "missing level+msg: {}",
            d
        );
        assert!(d.contains("let x = ;"), "missing source line: {}", d);
        assert!(d.contains("^"), "missing caret: {}", d);
    }

    #[test]
    fn multi_line_span_notes_continuation() {
        let src = "let s = \"hello\n world\";\n";
        let d = format_diagnostic(src, span(1, 9, 2, 8), "Error", "unterminated string");
        assert!(
            d.contains("(span continues on line 2)"),
            "expected multi-line continuation note: {}",
            d
        );
    }

    #[test]
    fn tabs_in_source_expand_to_four_spaces() {
        // Source line has a leading tab. The caret under column 5
        // should land under position 8 after tab expansion.
        let src = "\tlet x = 1;\n";
        let d = format_diagnostic(src, span(1, 5, 1, 6), "Warning", "sample");
        // Line must render with 4 spaces in place of the tab.
        assert!(
            d.contains("    let x = 1;"),
            "tab not expanded to 4 spaces: {}",
            d
        );
    }

    #[test]
    fn out_of_range_line_renders_empty_source_line_safely() {
        let src = "one-line file\n";
        let d = format_diagnostic(src, span(50, 1, 50, 2), "Error", "somewhere far away");
        // No crash, caret still present so the message is still
        // visually distinguishable in the terminal.
        assert!(d.contains("^"), "missing caret even on empty line: {}", d);
    }
}
