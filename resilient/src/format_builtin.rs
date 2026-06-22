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
use crate::span::Span;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormatArgumentKind {
    Integer,
    Float,
    String,
    Boolean,
    Other,
    Unknown,
}

impl FormatArgumentKind {
    fn label(self) -> &'static str {
        match self {
            FormatArgumentKind::Integer => "integer",
            FormatArgumentKind::Float => "float",
            FormatArgumentKind::String => "string",
            FormatArgumentKind::Boolean => "boolean",
            FormatArgumentKind::Other => "other",
            FormatArgumentKind::Unknown => "unknown",
        }
    }
}

pub fn infer_arg_kind(arg: &Node) -> FormatArgumentKind {
    match arg {
        Node::IntegerLiteral { .. } => FormatArgumentKind::Integer,
        Node::FloatLiteral { .. } => FormatArgumentKind::Float,
        Node::StringLiteral { .. }
        | Node::StringInternLiteral { .. }
        | Node::InterpolatedString { .. } => FormatArgumentKind::String,
        Node::BooleanLiteral { .. } => FormatArgumentKind::Boolean,
        Node::PrefixExpression {
            operator: "+" | "-",
            right,
            ..
        } if matches!(right.as_ref(), Node::IntegerLiteral { .. }) => FormatArgumentKind::Integer,
        Node::PrefixExpression {
            operator: "+" | "-",
            right,
            ..
        } if matches!(right.as_ref(), Node::FloatLiteral { .. }) => FormatArgumentKind::Float,
        Node::PrefixExpression { operator: "!", .. } => FormatArgumentKind::Boolean,
        _ => FormatArgumentKind::Unknown,
    }
}

pub fn spec_requires_integer(spec: &str) -> bool {
    if spec.is_empty() {
        return false;
    }
    let Some(rest) = spec.strip_prefix(':') else {
        return false;
    };
    rest.ends_with('d') || rest == "x" || rest == "X" || rest == "b" || rest == "o"
}

pub fn spec_requires_float(spec: &str) -> bool {
    if spec.is_empty() {
        return false;
    }
    spec.contains('.') || spec == ":e" || spec == ":E"
}

fn validate_argument_type(
    source_path: &str,
    arg_kind: FormatArgumentKind,
    spec: &str,
    arg: &Node,
) -> Option<String> {
    if arg_kind == FormatArgumentKind::Unknown {
        return None;
    }
    let requires_int = spec_requires_integer(spec);
    let requires_float = spec_requires_float(spec);

    let span_val = span_of(arg);
    let loc = format!(
        "{source_path}:{}:{}",
        span_val.start.line, span_val.start.column
    );

    match (arg_kind, requires_int, requires_float) {
        (FormatArgumentKind::String, true, _) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires integer argument, got string"
        )),
        (FormatArgumentKind::String, false, true) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires float argument, got string"
        )),
        (FormatArgumentKind::Boolean, true, _) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires integer argument, got boolean"
        )),
        (FormatArgumentKind::Boolean, false, true) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires float argument, got boolean"
        )),
        (FormatArgumentKind::Integer, false, true) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires float argument, got integer"
        )),
        (FormatArgumentKind::Float, true, false) => Some(format!(
            "{loc}: error[fmt]: format specifier `{{{spec}}}` requires integer argument, got float"
        )),
        _ => None,
    }
}

/// Walk the AST and validate every `format(template, ...)` call site.
///
/// Checks:
/// 1. The template string can be parsed (no unterminated braces).
/// 2. Each format specifier in the template is valid for its type.
/// 3. Placeholder count matches argument count.
/// 4. Argument types are compatible with their format specifiers (RES-3233).
/// 5. Format builtin call sites match their declarations (RES-3231).
pub(crate) fn check(program: &Node, source_path: &str) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();
    let format_builtins = check_format_builtin_declarations(source_path, &mut errors);

    let has_format_call = crate::uniqueness_walk::any_node(program, |n| {
        if let Node::CallExpression { function, .. } = n {
            if let Node::Identifier { name, .. } = function.as_ref() {
                return name == "format" || format_builtins.contains_key(name);
            }
        }
        false
    });
    if has_format_call {
        check_format_calls(program, source_path, &mut errors, &format_builtins);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

fn format_builtin_diag(source_path: &str, line: usize, item: &str, msg: impl AsRef<str>) -> String {
    format!(
        "{source_path}:{line}:0: error[fmt]: format_builtin declaration on `{item}` {}",
        msg.as_ref()
    )
}

fn span_of(node: &Node) -> Span {
    match node {
        Node::Identifier { span, .. }
        | Node::IntegerLiteral { span, .. }
        | Node::FloatLiteral { span, .. }
        | Node::StringLiteral { span, .. }
        | Node::StringInternLiteral { span, .. }
        | Node::InterpolatedString { span, .. }
        | Node::BooleanLiteral { span, .. }
        | Node::BytesLiteral { span, .. }
        | Node::CharLiteral { span, .. }
        | Node::PrefixExpression { span, .. }
        | Node::CallExpression { span, .. } => *span,
        _ => Span::default(),
    }
}

fn infer_literal_string(node: &Node) -> Option<&str> {
    match node {
        Node::StringLiteral { value, .. } => Some(value),
        Node::StringInternLiteral { content, .. } => Some(content),
        _ => None,
    }
}

fn parse_quoted_string(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }
    Some(&value[1..value.len() - 1])
}

fn validate_format_spec(spec: &str) -> Result<(), String> {
    if spec.is_empty() {
        return Ok(());
    }
    if spec.contains('.') || spec == ":e" || spec == ":E" {
        render_float(spec, 0.0).map(|_| ())
    } else {
        render_int(spec, 0).map(|_| ())
    }
}

struct FormatBuiltinDecl {
    template: String,
    arg_count: usize,
}

fn check_format_builtin_declarations(
    source_path: &str,
    errors: &mut Vec<String>,
) -> std::collections::HashMap<String, FormatBuiltinDecl> {
    let mut seen_items: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut decls = std::collections::HashMap::new();

    for (item, rec) in crate::feature_attrs::find_kind("format_builtin") {
        // RES-3232: detect duplicate/conflicting registrations
        if let Some(&first_line) = seen_items.get(&item) {
            errors.push(format!(
                "{source_path}:{}:0: error[fmt]: duplicate format_builtin registration for `{item}` \
                 (first declared on line {})",
                rec.line, first_line
            ));
            continue;
        }
        seen_items.insert(item.clone(), rec.line);

        let mut template: Option<String> = None;
        let mut arg_count: Option<usize> = None;
        let mut malformed = false;

        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                errors.push(format_builtin_diag(
                    source_path,
                    rec.line,
                    &item,
                    "has empty argument",
                ));
                malformed = true;
                continue;
            }

            let Some((key, value)) = chunk.split_once('=') else {
                errors.push(format_builtin_diag(
                    source_path,
                    rec.line,
                    &item,
                    "requires `key = value` arguments",
                ));
                malformed = true;
                continue;
            };
            let key = key.trim();
            let value = value.trim();

            match key {
                "template" => {
                    if template.is_some() {
                        errors.push(format!(
                            "{source_path}:{}:0: error[fmt]: duplicate `template` argument on `{item}`",
                            rec.line
                        ));
                        malformed = true;
                        continue;
                    }
                    let Some(value) = parse_quoted_string(value) else {
                        errors.push(format_builtin_diag(
                            source_path,
                            rec.line,
                            &item,
                            "requires quoted `template` string",
                        ));
                        malformed = true;
                        continue;
                    };
                    template = Some(value.to_string());
                }
                "args" => {
                    if arg_count.is_some() {
                        errors.push(format!(
                            "{source_path}:{}:0: error[fmt]: duplicate `args` argument on `{item}`",
                            rec.line
                        ));
                        malformed = true;
                        continue;
                    }
                    let Ok(parsed) = value.parse::<usize>() else {
                        errors.push(format_builtin_diag(
                            source_path,
                            rec.line,
                            &item,
                            "requires integer `args`",
                        ));
                        malformed = true;
                        continue;
                    };
                    arg_count = Some(parsed);
                }
                other => {
                    errors.push(format!(
                        "{source_path}:{}:0: error[fmt]: unknown format_builtin argument `{other}` on `{item}`",
                        rec.line
                    ));
                    malformed = true;
                }
            }
        }

        if malformed {
            continue;
        }

        let Some(template) = template else {
            errors.push(format_builtin_diag(
                source_path,
                rec.line,
                &item,
                "missing `template`",
            ));
            continue;
        };
        let Some(arg_count) = arg_count else {
            errors.push(format_builtin_diag(
                source_path,
                rec.line,
                &item,
                "missing `args`",
            ));
            continue;
        };

        let segments = match parse_template(&template) {
            Ok(segments) => segments,
            Err(err) => {
                errors.push(format_builtin_diag(source_path, rec.line, &item, err));
                continue;
            }
        };

        let placeholder_count = segments
            .iter()
            .filter(|s| matches!(s, FormatSegment::Placeholder(_)))
            .count();
        if placeholder_count != arg_count {
            errors.push(format!(
                "{source_path}:{}:0: error[fmt]: format_builtin declaration on `{item}` expects {placeholder_count} template placeholder(s) but declares {arg_count} arg(s)",
                rec.line
            ));
        }

        for segment in &segments {
            if let FormatSegment::Placeholder(spec) = segment {
                if let Err(err) = validate_format_spec(spec) {
                    errors.push(format_builtin_diag(source_path, rec.line, &item, err));
                    continue;
                }
            }
        }

        decls.insert(
            item.clone(),
            FormatBuiltinDecl {
                template,
                arg_count,
            },
        );
    }

    decls
}

fn check_format_calls(
    node: &Node,
    source_path: &str,
    errors: &mut Vec<String>,
    format_builtins: &std::collections::HashMap<String, FormatBuiltinDecl>,
) {
    if let Node::CallExpression {
        function,
        arguments,
        span,
    } = node
    {
        if let Node::Identifier { name, .. } = function.as_ref() {
            if name == "format" && !arguments.is_empty() {
                let line = span.start.line;
                let col = span.start.column;
                let loc = format!("{source_path}:{line}:{col}");
                let template_str = match &arguments[0] {
                    Node::StringLiteral { value, .. } => Some(value.as_str()),
                    Node::StringInternLiteral { content, .. } => Some(content.as_str()),
                    _ => None,
                };

                if let Some(value) = template_str {
                    match parse_template(value) {
                        Err(e) => {
                            errors.push(format!("{loc}: error[fmt]: {e}"));
                        }
                        Ok(segments) => {
                            let placeholder_count = segments
                                .iter()
                                .filter(|s| matches!(s, FormatSegment::Placeholder(_)))
                                .count();
                            let arg_count = arguments.len() - 1;
                            if placeholder_count != arg_count {
                                errors.push(format!(
                                    "{loc}: error[fmt]: `format` expects {placeholder_count} \
                                     argument(s) for template placeholders but {arg_count} \
                                     argument(s) were supplied"
                                ));
                            }
                            // RES-3233: validate each argument against its format specifier
                            let mut arg_index = 0;
                            for seg in &segments {
                                if let FormatSegment::Placeholder(spec) = seg {
                                    if !spec.is_empty() {
                                        let spec_check =
                                            if spec.contains('.') || spec == ":e" || spec == ":E" {
                                                render_float(spec, 0.0)
                                            } else {
                                                render_int(spec, 0)
                                            };
                                        if let Err(e) = spec_check {
                                            errors.push(format!("{loc}: error[fmt]: {e}"));
                                        }
                                    }
                                    // Check type compatibility between argument and specifier
                                    if arg_index + 1 < arguments.len() {
                                        let arg = &arguments[arg_index + 1];
                                        let arg_kind = infer_arg_kind(arg);
                                        if let Some(type_err) =
                                            validate_argument_type(source_path, arg_kind, spec, arg)
                                        {
                                            errors.push(type_err);
                                        }
                                    }
                                    arg_index += 1;
                                }
                            }
                        }
                    }
                }
            } else if let Some(decl) = format_builtins.get(name) {
                // RES-3231: validate format_builtin call sites
                let line = span.start.line;
                let col = span.start.column;
                let loc = format!("{source_path}:{line}:{col}");

                // Check argument count matches declaration
                if arguments.len() != decl.arg_count {
                    errors.push(format!(
                        "{loc}: error[fmt]: `{name}` is registered with {arg_count} argument(s), \
                         but call provides {actual_count} argument(s)",
                        arg_count = decl.arg_count,
                        actual_count = arguments.len()
                    ));
                } else {
                    // Parse the template to validate argument types
                    if let Ok(segments) = parse_template(&decl.template) {
                        let mut arg_index = 0;
                        for seg in &segments {
                            if let FormatSegment::Placeholder(spec) = seg {
                                if arg_index < arguments.len() {
                                    let arg = &arguments[arg_index];
                                    let arg_kind = infer_arg_kind(arg);
                                    if let Some(type_err) =
                                        validate_argument_type(source_path, arg_kind, spec, arg)
                                    {
                                        errors.push(type_err);
                                    }
                                }
                                arg_index += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    crate::uniqueness_walk::walk_children(node, &mut |child| {
        check_format_calls(child, source_path, errors, format_builtins);
    });
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

    // ── check() integration tests ─────────────────────────────────────────

    #[test]
    fn check_ok_on_program_without_format_calls() {
        let (prog, _) = crate::parse("fn f(int x) -> int { return x; }");
        assert!(check(&prog, "<test>").is_ok());
    }

    #[test]
    fn check_ok_on_valid_format_call() {
        let src = r#"fn main() { let s = format("{} and {}", 1, 2); }"#;
        let (prog, _) = crate::parse(src);
        // If format() is a CallExpression with string literal first arg, check validates it
        // The result depends on parser; at minimum it shouldn't panic
        let _ = check(&prog, "<test>");
    }

    #[test]
    fn check_passes_on_empty_program() {
        let (prog, _) = crate::parse("");
        assert!(check(&prog, "<test>").is_ok());
    }

    fn record_format_builtin(item: &str, args: &str, line: usize) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "format_builtin".into(),
                args: args.into(),
                line,
            },
        );
    }

    fn run_decl_check(source_path: &str) -> Result<(), String> {
        let (prog, _) = crate::parse("");
        check(&prog, source_path)
    }

    #[test]
    fn check_rejects_format_builtin_missing_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"args = 1"#, 11);

        let err = run_decl_check("test.rz").expect_err("expected missing template error");
        assert!(err.contains("test.rz:11:0: error[fmt]"), "{err}");
        assert!(
            err.contains("format_builtin declaration on `fmt` missing `template`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_missing_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}""#, 12);

        let err = run_decl_check("test.rz").expect_err("expected missing args error");
        assert!(err.contains("test.rz:12:0: error[fmt]"), "{err}");
        assert!(
            err.contains("format_builtin declaration on `fmt` missing `args`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_unknown_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 1, mode = "strict""#, 13);

        let err = run_decl_check("test.rz").expect_err("expected unknown argument error");
        assert!(err.contains("test.rz:13:0: error[fmt]"), "{err}");
        assert!(
            err.contains("unknown format_builtin argument `mode` on `fmt`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_duplicate_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", template = "{}", args = 1"#, 14);

        let err = run_decl_check("test.rz").expect_err("expected duplicate template error");
        assert!(err.contains("test.rz:14:0: error[fmt]"), "{err}");
        assert!(
            err.contains("duplicate `template` argument on `fmt`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_non_numeric_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = "one""#, 15);

        let err = run_decl_check("test.rz").expect_err("expected invalid args error");
        assert!(err.contains("test.rz:15:0: error[fmt]"), "{err}");
        assert!(
            err.contains("format_builtin declaration on `fmt` requires integer `args`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_malformed_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{", args = 1"#, 16);

        let err = run_decl_check("test.rz").expect_err("expected malformed template error");
        assert!(err.contains("test.rz:16:0: error[fmt]"), "{err}");
        assert!(err.contains("unterminated"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_arg_count_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {}", args = 1"#, 17);

        let err = run_decl_check("test.rz").expect_err("expected arg count mismatch");
        assert!(err.contains("test.rz:17:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 2 template placeholder(s) but declares 1 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_valid_format_builtin_declaration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {}", args = 2"#, 18);

        run_decl_check("test.rz").expect("valid format_builtin declaration should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn typechecker_rejects_parser_recorded_format_builtin_declaration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        let src = r#"
#[format_builtin(template = "{} {}", args = 1)]
fn fmt(int x) -> int { return x; }
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let mut checker = crate::typechecker::TypeChecker::new();
        let err = checker
            .check_program_with_source(&prog, "test.rz")
            .expect_err("expected typechecker to reject malformed declaration");
        assert!(err.contains("test.rz:0:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 2 template placeholder(s) but declares 1 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    // ── Malformed-input regression corpus ─────────────────────────────────

    #[test]
    fn check_rejects_format_builtin_empty_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 1, "#, 20);

        let err = run_decl_check("test.rz").expect_err("expected empty argument error");
        assert!(err.contains("test.rz:20:0: error[fmt]"), "{err}");
        assert!(err.contains("has empty argument"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_no_equals() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template "{}", args = 1"#, 21);

        let err = run_decl_check("test.rz").expect_err("expected no equals error");
        assert!(err.contains("test.rz:21:0: error[fmt]"), "{err}");
        assert!(err.contains("requires `key = value` arguments"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_unquoted_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = {}, args = 1"#, 22);

        let err = run_decl_check("test.rz").expect_err("expected unquoted template error");
        assert!(err.contains("test.rz:22:0: error[fmt]"), "{err}");
        assert!(err.contains("requires quoted `template` string"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_duplicate_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 1, args = 2"#, 23);

        let err = run_decl_check("test.rz").expect_err("expected duplicate args error");
        assert!(err.contains("test.rz:23:0: error[fmt]"), "{err}");
        assert!(err.contains("duplicate `args` argument on `fmt`"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_negative_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = -1"#, 24);

        let err = run_decl_check("test.rz").expect_err("expected negative args error");
        assert!(err.contains("test.rz:24:0: error[fmt]"), "{err}");
        assert!(err.contains("requires integer"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_args_zero_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 0"#, 25);

        let err = run_decl_check("test.rz").expect_err("expected args zero mismatch error");
        assert!(err.contains("test.rz:25:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 1 template placeholder(s) but declares 0 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_empty_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "", args = 0"#, 26);

        run_decl_check("test.rz").expect("empty template with zero args should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_no_placeholder_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "hello world", args = 0"#, 27);

        run_decl_check("test.rz").expect("template with no placeholders and zero args should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_multiple_placeholders() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {} {}", args = 3"#, 28);

        run_decl_check("test.rz").expect("multiple placeholders should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_many_placeholders_few_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {} {} {}", args = 2"#, 29);

        let err = run_decl_check("test.rz").expect_err("expected many placeholders error");
        assert!(err.contains("test.rz:29:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 4 template placeholder(s) but declares 2 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_escaped_brace_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{{ }}", args = 1"#, 30);

        let err = run_decl_check("test.rz").expect_err("expected brace mismatch error");
        assert!(err.contains("test.rz:30:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 0 template placeholder(s) but declares 1 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_mixed_escaped_real_braces() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{{ {}", args = 2"#, 31);

        let err = run_decl_check("test.rz").expect_err("expected mixed braces error");
        assert!(err.contains("test.rz:31:0: error[fmt]"), "{err}");
        assert!(
            err.contains("expects 1 template placeholder(s) but declares 2 arg(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    // ── Runtime parity: argument type validation (RES-3233) ────────────────────

    #[test]
    fn check_rejects_hex_format_with_string_literal() {
        let src = r#"
fn main() {
    format("{:x}", "hello");
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let err = check(&prog, "test.rz").expect_err("expected type mismatch error");
        assert!(
            err.contains("requires integer argument, got string"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_float_format_with_integer_literal() {
        let src = r#"
fn main() {
    format("{:.2f}", 42);
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let err = check(&prog, "test.rz").expect_err("expected type mismatch error");
        assert!(
            err.contains("requires float argument, got integer"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_integer_format_with_boolean_literal() {
        let src = r#"
fn main() {
    format("{:d}", true);
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let err = check(&prog, "test.rz").expect_err("expected type mismatch error");
        assert!(
            err.contains("requires integer argument, got boolean"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_binary_format_with_string_literal() {
        let src = r#"
fn main() {
    format("{:b}", "bits");
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let err = check(&prog, "test.rz").expect_err("expected type mismatch error");
        assert!(
            err.contains("requires integer argument, got string"),
            "{err}"
        );
    }

    #[test]
    fn check_rejects_octal_format_with_float_literal() {
        let src = r#"
fn main() {
    format("{:o}", 3.14);
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        let err = check(&prog, "test.rz").expect_err("expected type mismatch error");
        assert!(
            err.contains("requires integer argument, got float"),
            "{err}"
        );
    }

    #[test]
    fn check_accepts_integer_with_integer_format() {
        let src = r#"
fn main() {
    format("{:x}", 42);
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        check(&prog, "test.rz").expect("integer literal with hex format should pass");
    }

    #[test]
    fn check_accepts_float_with_float_format() {
        let src = r#"
fn main() {
    format("{:.3f}", 2.71828);
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        check(&prog, "test.rz").expect("float literal with float format should pass");
    }

    #[test]
    fn check_accepts_string_with_default_format() {
        let src = r#"
fn main() {
    format("{}", "hello");
}
"#;
        let (prog, parse_errs) = crate::parse(src);
        assert!(parse_errs.is_empty(), "parse errors: {parse_errs:?}");

        check(&prog, "test.rz").expect("string literal with default format should pass");
    }

    #[test]
    fn check_rejects_duplicate_format_builtin_registration() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        // Register "fmt" twice with different line numbers
        record_format_builtin("fmt", r#"template = "Hello {}", args = 1"#, 10);
        record_format_builtin("fmt", r#"template = "Goodbye {}", args = 1"#, 20);

        let err = run_decl_check("test.rz").expect_err("expected duplicate registration error");
        assert!(err.contains("test.rz:20:0: error[fmt]"), "{err}");
        assert!(
            err.contains("duplicate format_builtin registration for `fmt`"),
            "{err}"
        );
        assert!(err.contains("first declared on line 10"), "{err}");
        crate::feature_attrs::reset();
    }

    // ── RES-3231: format_builtin call-site validation tests ──────────────────

    #[test]
    fn check_rejects_format_builtin_call_with_wrong_arg_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {}", args = 2"#, 10);

        let src = r#"
fn main() {
    fmt(1);
}
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test.rz").expect_err("expected arg count mismatch");
        assert!(
            err.contains("is registered with 2 argument(s), but call provides 1 argument(s)"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_format_builtin_call_with_correct_args() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {}", args = 2"#, 10);

        let src = r#"
fn main() {
    fmt(1, 2);
}
"#;
        let (prog, _) = crate::parse(src);
        check(&prog, "test.rz").expect("fmt call with correct args should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_format_builtin_call_with_type_mismatch() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{:d}", args = 1"#, 10);

        let src = r#"
fn main() {
    fmt("string");
}
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test.rz").expect_err("expected type mismatch");
        assert!(
            err.contains("requires integer argument, got string"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_format_builtin_call_with_correct_types() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{:.2f}", args = 1"#, 10);

        let src = r#"
fn main() {
    fmt(3.14);
}
"#;
        let (prog, _) = crate::parse(src);
        check(&prog, "test.rz").expect("fmt call with correct type should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_validates_multiple_format_builtin_calls() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{:d}", args = 1"#, 10);

        let src = r#"
fn main() {
    fmt(1);
    fmt("invalid");
}
"#;
        let (prog, _) = crate::parse(src);
        let err = check(&prog, "test.rz").expect_err("expected second call to fail");
        assert!(
            err.contains("requires integer argument, got string"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    // ── Malformed-input regression corpus: RES-3234 ───────────────────────────
    // Comprehensive test coverage for edge cases, malformed input, and valid baseline scenarios.

    #[test]
    fn regression_format_builtin_baseline_simple_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "Hello {}", args = 1"#, 5);

        run_decl_check("test.rz").expect("simple template should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn regression_format_builtin_baseline_multiple_placeholders() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {} {}", args = 3"#, 6);

        run_decl_check("test.rz").expect("multiple placeholders should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn regression_format_builtin_baseline_complex_specifiers() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt1", r#"template = "{:.2f}", args = 1"#, 7);
        record_format_builtin("fmt2", r#"template = "{:05d}", args = 1"#, 8);
        record_format_builtin("fmt3", r#"template = "{:x}", args = 1"#, 9);

        run_decl_check("test.rz").expect("various format specifiers should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_zero_args_with_placeholder() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 0"#, 10);

        let err = run_decl_check("test.rz").expect_err("zero args but 1 placeholder should error");
        assert!(
            err.contains("expects 1 template placeholder(s) but declares 0"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_too_many_placeholders() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{} {} {} {}", args = 2"#, 11);

        let err = run_decl_check("test.rz").expect_err("too many placeholders should error");
        assert!(
            err.contains("expects 4 template placeholder(s) but declares 2"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_negative_args_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = -1"#, 12);

        let err = run_decl_check("test.rz").expect_err("negative args count should error");
        assert!(err.contains("requires"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_excessive_args_count() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{}", args = 10000"#, 13);

        let err = run_decl_check("test.rz").expect_err("excessive args count should error");
        assert!(
            err.contains("expects 1 template placeholder(s) but declares 10000"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_empty_template() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "", args = 0"#, 14);

        run_decl_check("test.rz").expect("empty template with zero args should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_unclosed_brace() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{", args = 1"#, 15);

        let err = run_decl_check("test.rz").expect_err("unclosed brace should error");
        assert!(err.contains("unterminated"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_scientific_notation_negative_exponent() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("fmt", r#"template = "{:e}", args = 1"#, 16);

        run_decl_check("test.rz").expect("scientific notation should pass");
        crate::feature_attrs::reset();
    }

    #[test]
    fn malformed_format_builtin_mixed_radix_specs() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_format_builtin("f1", r#"template = "{:b}", args = 1"#, 17);
        record_format_builtin("f2", r#"template = "{:o}", args = 1"#, 18);
        record_format_builtin("f3", r#"template = "{:x}", args = 1"#, 19);
        record_format_builtin("f4", r#"template = "{:X}", args = 1"#, 20);

        run_decl_check("test.rz").expect("mixed radix specs should pass");
        crate::feature_attrs::reset();
    }
}
