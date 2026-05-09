//! Feature 48/50 — `format()` Built-in.
//!
//! Adds a built-in `format(template, ...args)` that formats a string
//! at runtime. Format specifiers extend the existing string-
//! interpolation set:
//!
//! * `{}` — default Display
//! * `{:.Nf}` — float with N decimal places
//! * `{:Nd}` — int padded to width N
//! * `{:x}` / `{:X}` — hex (lower / upper case)
//!
//! The builtin parses the template and walks `args` in order.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormatSegment {
    Literal(String),
    Placeholder(String),
}

pub fn parse_template(s: &str) -> Vec<FormatSegment> {
    let mut out = Vec::new();
    let mut buf = String::new();
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
            let mut spec = String::new();
            while let Some(&c2) = chars.peek() {
                if c2 == '}' {
                    chars.next();
                    break;
                }
                spec.push(c2);
                chars.next();
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
    out
}

pub fn render_int(spec: &str, value: i64) -> String {
    if let Some(rest) = spec.strip_prefix(':') {
        if let Some(width) = rest.strip_suffix('d').and_then(|w| w.parse::<usize>().ok()) {
            return format!("{value:>width$}");
        }
        if rest == "x" {
            return format!("{value:x}");
        }
        if rest == "X" {
            return format!("{value:X}");
        }
    }
    value.to_string()
}

pub fn render_float(spec: &str, value: f64) -> String {
    if let Some(rest) = spec.strip_prefix(":.") {
        if let Some(prec) = rest.strip_suffix('f').and_then(|w| w.parse::<usize>().ok()) {
            return format!("{value:.prec$}");
        }
    }
    value.to_string()
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_template() {
        let segs = parse_template("Hello, {}!");
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
        let segs = parse_template("x = {:.2f}");
        assert_eq!(
            segs,
            vec![
                FormatSegment::Literal("x = ".into()),
                FormatSegment::Placeholder(":.2f".into()),
            ]
        );
    }

    #[test]
    fn render_float_with_precision() {
        assert_eq!(render_float(":.2f", 1.2345), "1.23");
    }

    #[test]
    fn render_int_hex() {
        assert_eq!(render_int(":x", 255), "ff");
        assert_eq!(render_int(":X", 255), "FF");
    }
}
