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
use std::fmt::Write as _;

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

    // RES-1984: pre-size to a close-fit estimate (header + line + carets
    // + optional multi-line tail). Skips the 0→4→...→256 doubling
    // cascade. `format_diagnostic` is on every compiler error/warning
    // path so the per-call savings are hot.
    let mut out = String::with_capacity(
        level.len()
            + 2
            + msg.len()
            + 1
            + 3
            + expanded.len()
            + 1
            + 3
            + pad.len()
            + carets.len()
            + 32,
    );
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
        // RES-1984: write! directly into `out` instead of the
        // `push_str(&format!())` antipattern. No intermediate String alloc.
        let _ = write!(out, "\n   (span continues on line {})", span.end.line);
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

/// RES-405 PR 2: wrap an error message that occurred during a generic
/// function's body with substitution context:
///
/// ```text
/// in generic fn<T = Int>: Type mismatch: 42 + hello
/// ```
///
/// Called by `apply_function` when the body eval fails and the function
/// has an `active_subst`. Produces a one-line prefix so the original
/// message (with its own `line:col:` if present) is preserved.
pub fn format_subst_context(
    fn_name: &str,
    subst: &crate::generics::Subst,
    original: &str,
) -> String {
    // RES-1984: collect pairs into a Vec for stable sort order, then
    // build the `args_str` directly via `write!` into a String — the
    // previous shape allocated a fresh `String` per pair via `format!()`
    // inside `.map(...)` only to drop those Strings after `.join(", ")`.
    let mut pairs: Vec<(&String, &crate::typechecker::Type)> = subst.iter().collect();
    pairs.sort_by_key(|(k, _)| k.as_str());

    let mut args_str = String::with_capacity(pairs.len() * 16);
    for (i, (k, v)) in pairs.iter().enumerate() {
        if i > 0 {
            args_str.push_str(", ");
        }
        let _ = write!(args_str, "{} = {}", k, v);
    }
    format!("in generic {}<{}>: {}", fn_name, args_str, original)
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
// RES-119: shared Diagnostic data model.
// ============================================================
//
// This section owns the typed diagnostic data structures and
// terminal renderer. Some call sites still emit `String` errors;
// follow-up migrations can adopt these types phase by phase.
//
// The types are `pub` so RES-206 (error-code registry) and
// later phase-migration tickets can consume them directly.

/// RES-119: severity lattice. Error > Warning > Hint > Note in
/// terms of user urgency. The terminal renderer prints the
/// lowercase name (`error:` / `warning:` / `hint:` / `note:`),
/// matching rustc's convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
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

#[allow(dead_code)]
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
///
/// **V2 TLA+ encoding requirement (RES-396):** This type MUST remain
/// a structured record with named fields, not a flat string. V2's
/// diagnostic projection into the TLA+ model extracts individual fields
/// (spec_path, action_name, trace_step, etc.) to encode verification
/// traces. Flattening diagnostics to strings would require V2 to
/// parse them back, defeating the purpose. Do not refactor this to
/// `pub struct Diagnostic(pub String)` or similar.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub code: Option<DiagCode>,
    pub message: String,
    pub notes: Vec<(Span, String)>,
}

#[allow(dead_code)]
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
    pub fn with_note(mut self, span: Span, message: impl Into<String>) -> Self {
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
#[allow(dead_code)]
pub fn format_diagnostic_terminal(src: &str, diag: &Diagnostic) -> String {
    // RES-1984: pre-size to a typical terminal-diagnostic size; switch
    // the three `push_str(&format!())` sites to `write!` for the header
    // and per-note headers.
    let mut out = String::with_capacity(256 + diag.notes.len() * 128);
    // Primary header: "<severity>[<code>]: <message>" — rustc-
    // shaped. Without a code, drop the brackets.
    match &diag.code {
        Some(code) => {
            let _ = writeln!(out, "{}[{}]: {}", diag.severity, code, diag.message);
        }
        None => {
            let _ = writeln!(out, "{}: {}", diag.severity, diag.message);
        }
    }
    // Source-context block for the primary span.
    out.push_str(&render_span_snippet(src, diag.span));
    // Each note: `note: <msg>` header + its own snippet block.
    for (note_span, note_msg) in &diag.notes {
        let _ = writeln!(out, "note: {}", note_msg);
        out.push_str(&render_span_snippet(src, *note_span));
    }
    out
}

/// RES-119 internal: extract just the snippet-with-caret
/// portion of `format_diagnostic`'s output, without its own
/// `<level>: <msg>` header. Lets
/// `format_diagnostic_terminal` own the header line in the new
/// `severity[code]:` shape.
#[allow(dead_code)]
fn render_span_snippet(src: &str, span: Span) -> String {
    // Re-use the existing renderer but strip its first line
    // (the `<level>: <msg>` header we don't want here).
    let full = format_diagnostic(src, span, "", "");
    // The first line is "`:` " (level empty + msg empty collapses
    // to "`: `"); drop it. If for any reason the helper returns
    // something unexpected, fall back to the raw output.
    full.lines().skip(1).collect::<Vec<_>>().join("\n") + "\n"
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

    // ---------- RES-119: Diagnostic model ----------

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
        let d = Diagnostic::new(Severity::Error, span(1, 1, 1, 2), "oops");
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
        let d = Diagnostic::new(Severity::Error, span(1, 10, 1, 10), "expected `;`")
            .with_code(DiagCode::new("E0007"));
        let out = format_diagnostic_terminal(src, &d);
        assert!(
            out.contains("error[E0007]: expected `;`"),
            "header wrong: {}",
            out
        );
        assert!(out.contains("let x = 1"), "source context missing: {}", out);
        assert!(out.contains("^"), "caret missing: {}", out);
    }

    #[test]
    fn terminal_renderer_without_code_drops_brackets() {
        let src = "let x = 1\n";
        let d = Diagnostic::new(Severity::Warning, span(1, 1, 1, 3), "unused binding");
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
        let d = Diagnostic::new(Severity::Error, primary, "shadows previous binding")
            .with_note(note_span, "previous definition here");
        let out = format_diagnostic_terminal(src, &d);
        assert!(
            out.contains("error: shadows previous binding"),
            "primary header missing: {}",
            out
        );
        assert!(
            out.contains("note: previous definition here"),
            "note header missing: {}",
            out
        );
        // Both source lines should appear (primary + note
        // snippets).
        assert!(
            out.contains("let x = 2;"),
            "primary snippet missing: {}",
            out
        );
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
        assert!(
            out.starts_with("hint: consider renaming to `_x`"),
            "header wrong: {}",
            out
        );
    }

    #[test]
    fn diagnostic_is_clone_and_eq() {
        // Clone + PartialEq derives exercised — matters for
        // downstream tests that want to assert on the full
        // Diagnostic value shape.
        let d1 = Diagnostic::new(Severity::Warning, span(1, 1, 1, 5), "x")
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
/// RES-206a landed the initial seed registry (10 codes) plus
/// sample docs pages. RES-4115 (E-E4, increment 1) extends the
/// registry with a second batch (E0011..E0020) covering common
/// declaration/type/runtime/verification diagnostics, each with
/// its own `docs/errors/E00NN.md` page, and adds the `rz explain
/// E00NN` CLI subcommand (`resilient/src/error_explain.rs`) that
/// renders those same pages in the terminal.
///
/// Auditing every existing diagnostic call site in `lib.rs` /
/// `typechecker.rs` and attaching a code at the point of
/// construction (without changing the rendered string shape that
/// `.expected.txt` goldens pin) is the next increment — most call
/// sites build a bare `String` error today, not a `Diagnostic`, so
/// that migration is call-site-by-call-site rather than mechanical.
/// A CI-enforceable lint that fails on a new codeless `Diagnostic`
/// literal, and generating `docs/errors/*.md` from this module
/// instead of hand-authoring it, follow once enough call sites are
/// migrated to make the lint meaningful.
///
/// Until that audit lands, most of these constants have no in-tree
/// callers beyond `E0007` (`typechecker.rs`) and the `T00NN`
/// prototype codes in `infer.rs` — the module-level
/// `#[allow(dead_code)]` keeps the build warning-clean.
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

    // ---- Name resolution / declarations ----

    /// E0011: Duplicate function definition — two `fn` declarations
    /// share the same name in the same scope.
    ///
    /// Docs: `docs/errors/E0011.html`.
    pub const E0011: DiagCode = DiagCode::new_static("E0011");

    /// E0012: Reassignment of an immutable (`let`) binding. Only
    /// `let mut` bindings may be reassigned after initialization.
    ///
    /// Docs: `docs/errors/E0012.html`.
    pub const E0012: DiagCode = DiagCode::new_static("E0012");

    // ---- Type checking ----

    /// E0013: Missing `return` on a code path in a function with a
    /// non-void declared return type.
    ///
    /// Docs: `docs/errors/E0013.html`.
    pub const E0013: DiagCode = DiagCode::new_static("E0013");

    /// E0014: Unwrap (`!` or `try`) of an `Optional` that resolved
    /// to `None` at runtime.
    ///
    /// Docs: `docs/errors/E0014.html`.
    pub const E0014: DiagCode = DiagCode::new_static("E0014");

    /// E0015: Import target not found — a `use`/`import` path
    /// doesn't resolve to a module or package on the search path.
    ///
    /// Docs: `docs/errors/E0015.html`.
    pub const E0015: DiagCode = DiagCode::new_static("E0015");

    /// E0016: Generic trait bound not satisfied — a type argument
    /// doesn't implement a bound required by the generic function
    /// or struct it's substituted into.
    ///
    /// Docs: `docs/errors/E0016.html`.
    pub const E0016: DiagCode = DiagCode::new_static("E0016");

    /// E0017: Unknown or missing struct field — a struct literal or
    /// field access names a field the struct definition doesn't have.
    ///
    /// Docs: `docs/errors/E0017.html`.
    pub const E0017: DiagCode = DiagCode::new_static("E0017");

    // ---- Runtime ----

    /// E0018: Recursion depth / stack usage limit exceeded.
    ///
    /// Docs: `docs/errors/E0018.html`.
    pub const E0018: DiagCode = DiagCode::new_static("E0018");

    // ---- Contracts / verification ----

    /// E0019: Z3 static verifier could not prove an `ensures` or
    /// `requires` clause ahead of time (distinct from E0010, which
    /// is the runtime check failing outright).
    ///
    /// Docs: `docs/errors/E0019.html`.
    pub const E0019: DiagCode = DiagCode::new_static("E0019");

    /// E0020: Effect/purity violation — a function annotated `pure`
    /// (or called from one) invokes a side-effecting operation.
    ///
    /// Docs: `docs/errors/E0020.html`.
    pub const E0020: DiagCode = DiagCode::new_static("E0020");

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
            E0001, E0002, E0003, E0004, E0005, E0006, E0007, E0008, E0009, E0010, E0011, E0012,
            E0013, E0014, E0015, E0016, E0017, E0018, E0019, E0020,
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
        let mut seen: Vec<String> = all.iter().map(|c| c.as_str().to_string()).collect();
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
        let strs: Vec<String> = all.iter().map(|c| c.as_str().to_string()).collect();
        for expected in &[
            "E0001", "E0002", "E0003", // parse
            "E0004", "E0005", "E0006", // name resolution
            "E0007", // type
            "E0008", "E0009", // runtime
            "E0010", // contracts
            "E0011", "E0012", // declarations
            "E0013", "E0014", "E0015", "E0016", "E0017", // type checking
            "E0018", // runtime
            "E0019", "E0020", // contracts / verification
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
        assert_eq!(codes::all().len(), 20);
    }

    #[test]
    fn res359_diagnostic_fields_are_public_and_named() {
        // RES-396: V2 TLA+ encoding requires Diagnostic to be a
        // structured record with named fields, not a flat string
        // or newtype. Create a Diagnostic and verify all public
        // fields are directly accessible by name.
        use crate::span::Pos;
        let pos = Pos::new(1, 1, 0);
        let test_span = Span::new(pos, pos);
        let d = Diagnostic::new(Severity::Error, test_span, "msg");
        // These field accesses verify the struct has named fields.
        // If Diagnostic were ever flattened to String, these would fail.
        let _span = d.span;
        let _severity = d.severity;
        let _code = d.code;
        let _message = d.message;
        let _notes = d.notes;
    }
}
