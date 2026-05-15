//! Feature 48/50 — `format()` Built-in.
//!
//! Adds a built-in `format(template, ...args)` that formats a string
//! at runtime. Format specifiers extend the existing string-
//! interpolation set:
//!
//! * `{}` — default Display
//! * `{:.Nf}` — float with N decimal places
//! * `{:e}` / `{:E}` — float in scientific notation (RES-1099)
//! * `{:Nd}` — int padded to width N (space-padded)
//! * `{:0Nd}` — int zero-padded to width N (RES-1097)
//! * `{:x}` / `{:X}` — hex (lower / upper case)
//! * `{:b}` — binary (RES-1094)
//! * `{:o}` — octal (RES-1095)
//!
//! The builtin parses the template and walks `args` in order. Unknown
//! specifiers and unterminated `{` placeholders are hard errors
//! (RES-1093 / RES-1096 / RES-1098) rather than silently producing
//! plausible-but-wrong output.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSegment {
    Literal(String),
    Placeholder(String),
}

/// Parse a `format()` template into a list of segments.
///
/// RES-1093: an unterminated `{` (no matching `}`) is now a hard
/// error. Previously the parser silently emitted an empty
/// `Placeholder("")`, masking malformed templates.
pub fn parse_template(s: &str) -> Result<Vec<FormatSegment>, String> {
    // RES-1816: fast-reject for templates with no placeholders and no
    // `}}` escape. The char-by-char loop below would walk every byte
    // only to deposit them all into a single trailing Literal. Format
    // strings without any `{` are extremely common (logging,
    // diagnostics, hard-coded messages); `fmt_validation` calls this
    // on every `format(...)` at typecheck time. The contains-`{` check
    // is one O(N) byte scan, far cheaper than the full chars-peekable
    // walk that follows on the slow path.
    if !s.contains('{') && !s.contains("}}") {
        return Ok(if s.is_empty() {
            Vec::new()
        } else {
            vec![FormatSegment::Literal(s.to_string())]
        });
    }
    // RES-1778: pre-size to (placeholder-count * 2 + 1) — at most one
    // Literal per `{...}` placeholder plus a trailing Literal, so this
    // matches the typical 1-3-placeholder shape. fmt_validation calls
    // this on every `format(...)` expression at typecheck time.
    let mut out = Vec::with_capacity(s.matches('{').count() * 2 + 1);
    // RES-1832: pre-size buf/spec to cover typical template segments
    // and format specifiers without realloc.
    let mut buf = String::with_capacity(16);
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                buf.push('{');
                chars.next();
                continue;
            }
            if !buf.is_empty() {
                out.push(FormatSegment::Literal(std::mem::take(&mut buf)));
            }
            let mut spec = String::with_capacity(8);
            let mut closed = false;
            while let Some(&c2) = chars.peek() {
                if c2 == '}' {
                    chars.next();
                    closed = true;
                    break;
                }
                spec.push(c2);
                chars.next();
            }
            if !closed {
                return Err(format!(
                    "format: unterminated `{{` in template (after `{}`)",
                    spec
                ));
            }
            out.push(FormatSegment::Placeholder(spec));
        } else if c == '}' && chars.peek() == Some(&'}') {
            buf.push('}');
            chars.next();
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        out.push(FormatSegment::Literal(buf));
    }
    Ok(out)
}

/// Render an integer through the standalone format-spec engine.
///
/// Supports `:Nd` (space-padded width), `:0Nd` (zero-padded width,
/// RES-1097), `:x`, `:X`, `:b` (RES-1094), `:o` (RES-1095). Unknown
/// specs return `Err` (RES-1096); previously they silently fell
/// through to default decimal output.
pub fn render_int(spec: &str, value: i64) -> Result<String, String> {
    if spec.is_empty() {
        return Ok(value.to_string());
    }
    let Some(rest) = spec.strip_prefix(':') else {
        return Err(format!("format: malformed integer spec `{}`", spec));
    };
    if let Some(width_str) = rest.strip_suffix('d') {
        let (zero_pad, width_digits) = if let Some(stripped) = width_str.strip_prefix('0') {
            (true, stripped)
        } else {
            (false, width_str)
        };
        let width: usize = width_digits
            .parse()
            .map_err(|_| format!("format: invalid integer width `{}`", width_str))?;
        return Ok(if zero_pad {
            if value < 0 {
                let body = format!(
                    "{:0>width$}",
                    value.unsigned_abs(),
                    width = width.saturating_sub(1)
                );
                format!("-{}", body)
            } else {
                format!("{value:0>width$}")
            }
        } else {
            format!("{value:>width$}")
        });
    }
    match rest {
        "x" => Ok(if value < 0 {
            format!("-{:x}", value.unsigned_abs())
        } else {
            format!("{value:x}")
        }),
        "X" => Ok(if value < 0 {
            format!("-{:X}", value.unsigned_abs())
        } else {
            format!("{value:X}")
        }),
        "b" => Ok(if value < 0 {
            format!("-{:b}", value.unsigned_abs())
        } else {
            format!("{value:b}")
        }),
        "o" => Ok(if value < 0 {
            format!("-{:o}", value.unsigned_abs())
        } else {
            format!("{value:o}")
        }),
        other => Err(format!("format: unknown integer spec `:{}`", other)),
    }
}

/// Render a float through the standalone format-spec engine.
///
/// Supports `:.Nf` (precision), `:e` / `:E` (scientific notation,
/// RES-1099). Unknown specs return `Err` (RES-1098); previously they
/// silently fell through to default `to_string()`.
pub fn render_float(spec: &str, value: f64) -> Result<String, String> {
    if spec.is_empty() {
        return Ok(value.to_string());
    }
    if let Some(rest) = spec.strip_prefix(":.") {
        if let Some(prec_str) = rest.strip_suffix('f') {
            let prec: usize = prec_str
                .parse()
                .map_err(|_| format!("format: invalid float precision `{}`", prec_str))?;
            return Ok(format!("{value:.prec$}"));
        }
        return Err(format!("format: malformed float spec `{}`", spec));
    }
    match spec {
        ":e" => Ok(format!("{value:e}")),
        ":E" => Ok(format!("{value:E}")),
        other => Err(format!("format: unknown float spec `{}`", other)),
    }
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_template() {
        let segs = parse_template("Hello, {}!").unwrap();
        assert_eq!(
            segs,
            vec![
                FormatSegment::Literal("Hello, ".into()),
                FormatSegment::Placeholder("".into()),
                FormatSegment::Literal("!".into()),
            ]
        );
    }

    #[test]
    fn parse_with_spec() {
        let segs = parse_template("x = {:.2f}").unwrap();
        assert_eq!(
            segs,
            vec![
                FormatSegment::Literal("x = ".into()),
                FormatSegment::Placeholder(":.2f".into()),
            ]
        );
    }

    /// RES-1093: unterminated `{` is a hard error, not a silent
    /// empty-placeholder.
    #[test]
    fn parse_unterminated_brace_errors() {
        let err = parse_template("hello {").unwrap_err();
        assert!(err.contains("unterminated"), "got: {err}");
    }

    /// RES-1093: unterminated `{:spec` (with content but no closer)
    /// is also an error.
    #[test]
    fn parse_unterminated_brace_with_spec_errors() {
        let err = parse_template("x = {:.2f").unwrap_err();
        assert!(err.contains("unterminated"), "got: {err}");
    }

    #[test]
    fn render_float_with_precision() {
        assert_eq!(render_float(":.2f", 1.2345).unwrap(), "1.23");
    }

    #[test]
    fn render_int_hex() {
        assert_eq!(render_int(":x", 255).unwrap(), "ff");
        assert_eq!(render_int(":X", 255).unwrap(), "FF");
    }

    /// RES-1094: binary radix is supported.
    #[test]
    fn render_int_binary() {
        assert_eq!(render_int(":b", 10).unwrap(), "1010");
        assert_eq!(render_int(":b", -5).unwrap(), "-101");
    }

    /// RES-1095: octal radix is supported.
    #[test]
    fn render_int_octal() {
        assert_eq!(render_int(":o", 8).unwrap(), "10");
        assert_eq!(render_int(":o", 64).unwrap(), "100");
        assert_eq!(render_int(":o", -9).unwrap(), "-11");
    }

    /// RES-1096: unknown radix is an error, not silent fall-through.
    #[test]
    fn render_int_unknown_spec_errors() {
        let err = render_int(":q", 10).unwrap_err();
        assert!(err.contains("unknown integer spec"), "got: {err}");
    }

    /// RES-1097: leading `0` in width triggers zero-padding.
    #[test]
    fn render_int_zero_padding() {
        assert_eq!(render_int(":05d", 7).unwrap(), "00007");
        assert_eq!(render_int(":05d", -3).unwrap(), "-0003");
        assert_eq!(render_int(":3d", 7).unwrap(), "  7");
    }

    /// RES-1098: unknown float spec is an error.
    #[test]
    fn render_float_unknown_spec_errors() {
        let err = render_float(":q", 1.5).unwrap_err();
        assert!(err.contains("unknown float spec"), "got: {err}");
    }

    /// RES-1099: scientific-notation specifier is supported.
    #[test]
    fn render_float_scientific() {
        assert_eq!(render_float(":e", 1500.0).unwrap(), "1.5e3");
        assert_eq!(render_float(":E", 1500.0).unwrap(), "1.5E3");
    }

    #[test]
    fn render_int_invalid_width_errors() {
        let err = render_int(":xxd", 1).unwrap_err();
        assert!(err.contains("invalid integer width"), "got: {err}");
    }
}
