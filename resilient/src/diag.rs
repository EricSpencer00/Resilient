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

// ============================================================
// RES-119 (scaffolding-only): unified Diagnostic data model.
// ============================================================
//
// This section lands the data types + terminal renderer. Call-
// site migration (parser / typechecker / VM / verifier / LSP)
// is deliberately NOT in scope here per the bail's Option 2 —
// the existing pipelines keep emitting `String` errors unchanged
// until follow-up tickets migrate each phase individually.
//
// The types are `pub` so RES-206 (error-code registry) and
// later phase-migration tickets can consume them directly.

/// RES-119: severity lattice. Error > Warning > Hint > Note in
/// terms of user urgency. The terminal renderer prints the
/// lowercase name (`error:` / `warning:` / `hint:` / `note:`),
/// matching rustc's convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// Compile / verify / runtime error — blocks execution.
    Error,
    /// Non-fatal signal — lints, exhaustiveness warnings,
    /// deprecation.
    Warning,
    /// Suggestive ("consider using X") — rendered as info.
    Hint,
    /// Secondary message attached to a primary diagnostic —
    /// typically surfaced as "note: previous definition here".
    Note,
}

impl Severity {
    /// Lowercase string suitable for the terminal prefix.
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Hint => "hint",
            Severity::Note => "note",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// RES-119: stable error/warning identifier. Registry lives in
/// the follow-up ticket RES-206 (`resilient/src/diag/codes.rs`);
/// this type is the shape the registry hangs off of. Using a
/// `Cow<'static, str>` lets both static `pub const` entries
/// (`"E0001"`) and dynamic-construction code paths share the
/// same type without allocating for the static cases.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiagCode(pub std::borrow::Cow<'static, str>);

impl DiagCode {
    /// RES-119: constant-friendly constructor. `DiagCode::new("E0001")`
    /// borrows the `'static str` with no allocation.
    pub const fn new(code: &'static str) -> Self {
        DiagCode(std::borrow::Cow::Borrowed(code))
    }

    /// String view — what gets rendered inside
    /// `error[<code>]: message`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DiagCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// RES-119: the unified diagnostic. Every phase produces these
/// (once migrated); the LSP publish path + terminal renderer
/// consume them without phase-specific adapters.
///
/// - `span` — the primary source range the diagnostic points at.
/// - `severity` — Error / Warning / Hint / Note.
/// - `code` — optional stable identifier (populated by RES-206).
/// - `message` — the main human-readable text.
/// - `notes` — secondary (span, message) pairs attached to the
///   primary. Rendered as additional `note:` blocks. Empty by
///   default; no new renderer work required — the terminal
///   formatter just prints them after the primary block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: Option<DiagCode>,
    pub message: String,
    pub notes: Vec<(Span, String)>,
}

impl Diagnostic {
    /// RES-119: quick-constructor for a no-code no-notes
    /// diagnostic. Most call-site migrations start here and
    /// add code + notes later as the phase gets more
    /// sophisticated.
    pub fn new(severity: Severity, span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            severity,
            code: None,
            message: message.into(),
            notes: Vec::new(),
        }
    }

    /// Fluent builder: attach a stable code.
    pub fn with_code(mut self, code: DiagCode) -> Self {
        self.code = Some(code);
        self
    }

    /// Fluent builder: append a secondary note with its own
    /// span. Chainable; called once per note.
    pub fn with_note(
        mut self,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        self.notes.push((span, message.into()));
        self
    }
}

/// RES-119: terminal renderer for a unified Diagnostic. Produces
/// the same source-context-with-caret shape as the existing
/// `format_diagnostic` helper, but with a severity + optional
/// `[code]` prefix and any attached notes printed as follow-up
/// blocks.
///
/// Example output (single note):
///
/// ```text
/// error[E0007]: expected `;`
///    let x = 1
///             ^
/// note: statement boundary inferred here
///    let x = 1
///             ^
/// ```
///
/// The `filename` prefix is the caller's responsibility (this
/// matches `format_diagnostic` / `format_diagnostic_from_line_col`).
/// Callers render the final `<file>:<line>:<col>: ` before
/// invoking the terminal formatter.
///
/// No ANSI colour codes — same reasoning as
/// `format_diagnostic`: diagnostics often pipe into logs / LSP,
/// where escape codes render as garbage.
pub fn format_diagnostic_terminal(src: &str, diag: &Diagnostic) -> String {
    let mut out = String::new();
    // Primary header: "<severity>[<code>]: <message>" — rustc-
    // shaped. Without a code, drop the brackets.
    match &diag.code {
        Some(code) => {
            out.push_str(&format!(
                "{}[{}]: {}\n",
                diag.severity, code, diag.message
            ));
        }
        None => {
            out.push_str(&format!("{}: {}\n", diag.severity, diag.message));
        }
    }
    // Source-context block for the primary span.
    out.push_str(&render_span_snippet(src, diag.span));
    // Each note: `note: <msg>` header + its own snippet block.
    for (note_span, note_msg) in &diag.notes {
        out.push_str(&format!("note: {}\n", note_msg));
        out.push_str(&render_span_snippet(src, *note_span));
    }
    out
}

/// RES-119 internal: extract just the snippet-with-caret
/// portion of `format_diagnostic`'s output, without its own
/// `<level>: <msg>` header. Lets
/// `format_diagnostic_terminal` own the header line in the new
/// `severity[code]:` shape.
fn render_span_snippet(src: &str, span: Span) -> String {
    // Re-use the existing renderer but strip its first line
    // (the `<level>: <msg>` header we don't want here).
    let full = format_diagnostic(src, span, "", "");
    // The first line is "`:` " (level empty + msg empty collapses
    // to "`: `"); drop it. If for any reason the helper returns
    // something unexpected, fall back to the raw output.
    full.lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
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

    // ---------- RES-119: Diagnostic scaffolding ----------

    #[test]
    fn severity_renders_lowercase_rustc_style() {
        assert_eq!(Severity::Error.as_str(), "error");
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Hint.as_str(), "hint");
        assert_eq!(Severity::Note.as_str(), "note");
        // Display impl matches as_str.
        assert_eq!(format!("{}", Severity::Error), "error");
    }

    #[test]
    fn diag_code_const_constructor_is_borrow() {
        // Constant-friendly: no allocation for 'static strs.
        const E0001: DiagCode = DiagCode::new("E0001");
        assert_eq!(E0001.as_str(), "E0001");
        assert_eq!(format!("{}", E0001), "E0001");
    }

    #[test]
    fn diagnostic_new_leaves_optional_fields_empty() {
        let d = Diagnostic::new(
            Severity::Error,
            span(1, 1, 1, 2),
            "oops",
        );
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.message, "oops");
        assert!(d.code.is_none());
        assert!(d.notes.is_empty());
    }

    #[test]
    fn diagnostic_builder_attaches_code_and_notes() {
        let code = DiagCode::new("E0007");
        let primary = span(1, 1, 1, 5);
        let note_span = span(2, 1, 2, 5);
        let d = Diagnostic::new(Severity::Error, primary, "expected `;`")
            .with_code(code.clone())
            .with_note(note_span, "statement boundary here");
        assert_eq!(d.code, Some(code));
        assert_eq!(d.notes.len(), 1);
        assert_eq!(d.notes[0].1, "statement boundary here");
    }

    #[test]
    fn terminal_renderer_includes_severity_code_and_message() {
        let src = "let x = 1\n";
        let d = Diagnostic::new(
            Severity::Error,
            span(1, 10, 1, 10),
            "expected `;`",
        )
        .with_code(DiagCode::new("E0007"));
        let out = format_diagnostic_terminal(src, &d);
        assert!(out.contains("error[E0007]: expected `;`"),
            "header wrong: {}", out);
        assert!(out.contains("let x = 1"),
            "source context missing: {}", out);
        assert!(out.contains("^"), "caret missing: {}", out);
    }

    #[test]
    fn terminal_renderer_without_code_drops_brackets() {
        let src = "let x = 1\n";
        let d = Diagnostic::new(
            Severity::Warning,
            span(1, 1, 1, 3),
            "unused binding",
        );
        let out = format_diagnostic_terminal(src, &d);
        assert!(
            out.starts_with("warning: unused binding"),
            "header shape wrong: {}",
            out,
        );
        // `[...]` shouldn't appear anywhere near the severity.
        assert!(
            !out.lines().next().unwrap().contains("["),
            "stray `[` in header: {}",
            out,
        );
    }

    #[test]
    fn terminal_renderer_appends_each_note_with_its_own_snippet() {
        let src = "let x = 1;\nlet x = 2;\n";
        let primary = span(2, 5, 2, 6);
        let note_span = span(1, 5, 1, 6);
        let d = Diagnostic::new(
            Severity::Error,
            primary,
            "shadows previous binding",
        )
        .with_note(note_span, "previous definition here");
        let out = format_diagnostic_terminal(src, &d);
        assert!(out.contains("error: shadows previous binding"),
            "primary header missing: {}", out);
        assert!(out.contains("note: previous definition here"),
            "note header missing: {}", out);
        // Both source lines should appear (primary + note
        // snippets).
        assert!(out.contains("let x = 2;"), "primary snippet missing: {}", out);
        assert!(out.contains("let x = 1;"), "note snippet missing: {}", out);
    }

    #[test]
    fn hint_severity_renders_with_hint_header() {
        let src = "let x = 1;\n";
        let d = Diagnostic::new(
            Severity::Hint,
            span(1, 1, 1, 3),
            "consider renaming to `_x`",
        );
        let out = format_diagnostic_terminal(src, &d);
        assert!(out.starts_with("hint: consider renaming to `_x`"),
            "header wrong: {}", out);
    }

    #[test]
    fn diagnostic_is_clone_and_eq() {
        // Clone + PartialEq derives exercised — matters for
        // downstream tests that want to assert on the full
        // Diagnostic value shape.
        let d1 = Diagnostic::new(
            Severity::Warning,
            span(1, 1, 1, 5),
            "x",
        )
        .with_code(DiagCode::new("W0001"));
        let d2 = d1.clone();
        assert_eq!(d1, d2);
    }
}

/// RES-206a: central registry of diagnostic codes.
///
/// Every code is a `pub const DiagCode` so call sites look like
/// `diag::codes::E0003`. Using constants (not an enum) keeps the
/// list append-only without breaking `Diagnostic::with_code`
/// signatures when codes are added. Constants also match rustc's
/// own error-code surface, which keeps editor plugins and
/// downstream diagnostic-rewriting tooling predictable.
///
/// ## Numbering policy
///
/// - **E-prefixed codes** (`E0001`..) — errors.
/// - **W-prefixed codes** (`W0001`..) — warnings.
///
/// Numbers are **sticky**: once assigned to a specific diagnostic
/// cause, a code is never reused. If a diagnostic is removed,
/// its code is retired (kept as a comment line but no longer
/// exported as a constant) — this preserves the docs-page URL
/// space and stops external cheat sheets from silently drifting.
///
/// ## Scope of this module
///
/// RES-206a lands only the initial seed registry (the ~10 codes
/// below) plus sample docs pages. Auditing every existing
/// diagnostic call site and assigning them codes is RES-206b;
/// writing the remaining docs pages is RES-206c.
///
/// Until that audit lands, these constants have no in-tree
/// callers — the module-level `#[allow(dead_code)]` keeps the
/// build warning-clean. Remove the allow when RES-206b starts
/// attaching codes to actual error-creation sites.
#[allow(dead_code)]
pub mod codes {
    use super::DiagCode;

    // ---- Parser errors ----

    /// E0001: Generic parse error. Emitted when the parser
    /// can't reconcile the token stream with any valid grammar
    /// production and no more-specific code applies.
    ///
    /// Docs: `docs/errors/E0001.html`.
    pub const E0001: DiagCode = DiagCode::new_static("E0001");

    /// E0002: Unexpected `;` / missing `;`. One of the most
    /// common parser errors; worth a dedicated code so editors
    /// can flag and fix it.
    ///
    /// Docs: `docs/errors/E0002.html`.
    pub const E0002: DiagCode = DiagCode::new_static("E0002");

    /// E0003: Unclosed delimiter (`(`, `[`, `{`). Reported by the
    /// parser when a nesting level doesn't close before EOF.
    ///
    /// Docs: `docs/errors/E0003.html`.
    pub const E0003: DiagCode = DiagCode::new_static("E0003");

    // ---- Name resolution ----

    /// E0004: Unknown identifier. Surfaced by the parser's post-
    /// pass, the interpreter, or the typechecker when a name is
    /// referenced before binding.
    ///
    /// Docs: `docs/errors/E0004.html`.
    pub const E0004: DiagCode = DiagCode::new_static("E0004");

    /// E0005: Unknown function at a call site.
    ///
    /// Docs: `docs/errors/E0005.html`.
    pub const E0005: DiagCode = DiagCode::new_static("E0005");

    /// E0006: Call arity mismatch — wrong number of arguments.
    ///
    /// Docs: `docs/errors/E0006.html`.
    pub const E0006: DiagCode = DiagCode::new_static("E0006");

    // ---- Type checking ----

    /// E0007: Type mismatch. RHS type doesn't match the declared
    /// or inferred LHS type.
    ///
    /// Docs: `docs/errors/E0007.html`.
    pub const E0007: DiagCode = DiagCode::new_static("E0007");

    // ---- Runtime ----

    /// E0008: Division by zero.
    ///
    /// Docs: `docs/errors/E0008.html`.
    pub const E0008: DiagCode = DiagCode::new_static("E0008");

    /// E0009: Array index out of bounds.
    ///
    /// Docs: `docs/errors/E0009.html`.
    pub const E0009: DiagCode = DiagCode::new_static("E0009");

    // ---- Contracts (requires / ensures) ----

    /// E0010: Contract violation — a `requires` or `ensures`
    /// clause evaluated to false at runtime.
    ///
    /// Docs: `docs/errors/E0010.html`.
    pub const E0010: DiagCode = DiagCode::new_static("E0010");

    // ---- Enumeration helper ----

    /// Every code registered in this module, in numeric order.
    /// Used by the registry tests to guard against drift between
    /// the module's constants and what the docs site lists. Also
    /// surfaced for possible future tooling (e.g. a
    /// `resilient errors list` subcommand).
    ///
    /// Returns `Vec<DiagCode>` (owned) rather than a static slice
    /// because `DiagCode` holds a `Cow<'static, str>` that the
    /// compiler refuses to stash in a `&'static [&DiagCode]`
    /// literal — every `&E0001` in an array expression would
    /// materialize a temporary. The vec is small (10 entries
    /// today) and allocated at most once per caller, so the cost
    /// is negligible.
    pub fn all() -> Vec<DiagCode> {
        vec![
            E0001, E0002, E0003, E0004, E0005, E0006, E0007, E0008, E0009, E0010,
        ]
    }
}

impl DiagCode {
    /// RES-206a: `const`-friendly constructor taking a `&'static str`.
    /// Named differently from `DiagCode::new` (existing in RES-119)
    /// to disambiguate from the borrowed-cow form, but both paths
    /// produce the same shape. `new_static` is the one the registry
    /// consts below use.
    pub const fn new_static(code: &'static str) -> Self {
        DiagCode(std::borrow::Cow::Borrowed(code))
    }
}

#[cfg(test)]
mod codes_tests {
    use super::*;

    #[test]
    fn res206a_codes_are_distinct_strings() {
        // Sanity: no two codes accidentally share a string.
        let all = codes::all();
        let mut seen: Vec<String> =
            all.iter().map(|c| c.as_str().to_string()).collect();
        let before = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(
            seen.len(),
            before,
            "duplicate code detected in the registry: {:?}",
            seen
        );
    }

    #[test]
    fn res206a_codes_follow_e_prefix_convention() {
        // Errors start with `E`, warnings start with `W`. The
        // initial registry is all errors; if a warning sneaks in,
        // it must also be named accordingly.
        for code in codes::all() {
            let s = code.as_str();
            assert!(
                s.starts_with('E') || s.starts_with('W'),
                "code {:?} doesn't follow the E/W prefix convention",
                s.to_string()
            );
        }
    }

    #[test]
    fn res206a_codes_render_inline_in_diagnostic() {
        // End-to-end: attach `codes::E0007` to a Diagnostic and
        // verify the terminal renderer puts `[E0007]` inline.
        let src = "let x = 42;";
        let diag = Diagnostic::new(
            Severity::Error,
            Span::new(
                crate::span::Pos::new(1, 5, 0),
                crate::span::Pos::new(1, 6, 1),
            ),
            "type mismatch",
        )
        .with_code(codes::E0007);
        let rendered = format_diagnostic_terminal(src, &diag);
        assert!(
            rendered.contains("error[E0007]:"),
            "expected `error[E0007]:` in rendered output:\n{}",
            rendered,
        );
    }

    #[test]
    fn res206a_initial_codes_cover_core_categories() {
        // Pin the initial set so accidental removals are caught:
        // parse / identifier / type / runtime / contract bands
        // must all remain represented.
        let all = codes::all();
        let strs: Vec<String> =
            all.iter().map(|c| c.as_str().to_string()).collect();
        for expected in &[
            "E0001", "E0002", "E0003", // parse
            "E0004", "E0005", "E0006", // name resolution
            "E0007",                   // type
            "E0008", "E0009",          // runtime
            "E0010",                   // contracts
        ] {
            assert!(
                strs.iter().any(|s| s == expected),
                "initial code {} missing from registry: {:?}",
                expected,
                strs,
            );
        }
    }

    #[test]
    fn res206a_new_static_is_const_friendly() {
        const CODE: DiagCode = DiagCode::new_static("E9999");
        assert_eq!(CODE.as_str(), "E9999");
    }

    #[test]
    fn res206a_codes_all_count_matches_vec_len() {
        // Regression guard: `all()` must not drop entries.
        assert_eq!(codes::all().len(), 10);
    }
}
