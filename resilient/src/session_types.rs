//! Feature 20/50 - Session Types.
//!
//! `#[session(protocol = "send(int).recv(bool).close")]` declares
//! a protocol-typed channel: sequence operations must be
//! performed in order. Calls that violate the sequence are
//! rejected at compile time.
//!
//! Protocol grammar (string-encoded for now): operations are
//! separated by `.`. Each operation is one of:
//! * `send(T)` - caller sends a value of type `T`
//! * `recv(T)` - caller receives a value of type `T`
//! * `close` - terminates the protocol
//!
//! This module records protocol definitions and exposes a
//! `next_op(channel, operation)` API that runtime / typechecker
//! logic consults during validation.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOp {
    Send(String),
    Recv(String),
    Close,
}

#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub protocol: Vec<SessionOp>,
}

static SPECS: LazyLock<RwLock<HashMap<String, SessionSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn diag(line: usize, msg: impl AsRef<str>) -> String {
    format!("{}:1: {}", line.saturating_add(1), msg.as_ref())
}

fn split_attr_args(args: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    let mut escaped = false;

    for (idx, ch) in args.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(args[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }

    parts.push(args[start..].trim());
    parts
}

fn parse_quoted_string(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return None;
    }
    Some(&value[1..value.len() - 1])
}

fn parse_session_protocol_arg(
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<String, String> {
    let mut protocol = None;
    let chunks = split_attr_args(&rec.args);

    if chunks.len() == 1 && chunks[0].is_empty() {
        return Err(diag(
            rec.line,
            format!("#[session] on `{item}` is missing `protocol = \"...\"`"),
        ));
    }

    for chunk in chunks {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            return Err(diag(
                rec.line,
                format!("empty `#[session]` argument on `{item}`"),
            ));
        }

        let Some((key, value)) = chunk.split_once('=') else {
            return Err(diag(
                rec.line,
                format!("invalid `#[session]` argument `{chunk}` on `{item}`"),
            ));
        };

        let key = key.trim();
        let value = value.trim();
        if key != "protocol" {
            return Err(diag(
                rec.line,
                format!("unknown `#[session]` argument `{key}` on `{item}`"),
            ));
        }
        if protocol.is_some() {
            return Err(diag(
                rec.line,
                format!("duplicate `protocol` argument on `{item}`"),
            ));
        }

        let Some(parsed) = parse_quoted_string(value) else {
            return Err(diag(
                rec.line,
                format!("`protocol` on `{item}` must be a quoted string"),
            ));
        };
        protocol = Some(parsed.to_string());
    }

    protocol.ok_or_else(|| {
        diag(
            rec.line,
            format!("#[session] on `{item}` is missing `protocol = \"...\"`"),
        )
    })
}

fn parse_protocol_strict(item: &str, s: &str, line: usize) -> Result<Vec<SessionOp>, String> {
    let mut out = Vec::new();
    let mut saw_close = false;

    for (idx, raw) in s.split('.').enumerate() {
        let op = raw.trim();
        if op.is_empty() {
            return Err(diag(
                line,
                format!("empty session protocol step {} on `{item}`", idx + 1),
            ));
        }
        if saw_close {
            return Err(diag(
                line,
                format!("session protocol on `{item}` cannot continue after `close`"),
            ));
        }

        if op == "close" {
            out.push(SessionOp::Close);
            saw_close = true;
            continue;
        }

        if let Some(rest) = op.strip_prefix("send(") {
            if let Some(t) = rest.strip_suffix(')') {
                let t = t.trim();
                if t.is_empty() {
                    return Err(diag(
                        line,
                        format!("`send(...)` on `{item}` needs a type name"),
                    ));
                }
                out.push(SessionOp::Send(t.to_string()));
                continue;
            }
        }

        if let Some(rest) = op.strip_prefix("recv(") {
            if let Some(t) = rest.strip_suffix(')') {
                let t = t.trim();
                if t.is_empty() {
                    return Err(diag(
                        line,
                        format!("`recv(...)` on `{item}` needs a type name"),
                    ));
                }
                out.push(SessionOp::Recv(t.to_string()));
                continue;
            }
        }

        return Err(diag(
            line,
            format!("invalid session protocol step `{op}` on `{item}`"),
        ));
    }

    if !matches!(out.last(), Some(SessionOp::Close)) {
        return Err(diag(
            line,
            format!("session protocol on `{item}` must end with `close`"),
        ));
    }

    Ok(out)
}

fn validate_session_record(
    item: &str,
    rec: &crate::feature_attrs::AttrRecord,
) -> Result<(String, SessionSpec), String> {
    let proto = parse_session_protocol_arg(item, rec)?;
    let protocol = parse_protocol_strict(item, &proto, rec.line)?;
    Ok((item.to_string(), SessionSpec { protocol }))
}

pub fn parse_protocol(s: &str) -> Vec<SessionOp> {
    parse_protocol_strict("", s, 0).unwrap_or_default()
}

pub fn collect() -> Vec<(String, SessionSpec)> {
    let attrs = crate::feature_attrs::find_kind("session");
    let mut out = Vec::with_capacity(attrs.len());

    for (item, rec) in attrs {
        if let Ok(spec) = validate_session_record(&item, &rec) {
            out.push(spec);
        }
    }

    out
}

pub fn install(specs: Vec<(String, SessionSpec)>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        g.extend(specs);
    }
}

pub fn validate_step(channel: &str, step: usize, op: &SessionOp) -> Result<(), String> {
    let g = SPECS
        .read()
        .map_err(|_| format!("no session protocol `{channel}`"))?;
    let spec = g
        .get(channel)
        .ok_or_else(|| format!("no session protocol `{channel}`"))?;
    let expected = spec.protocol.get(step).ok_or_else(|| {
        format!("session protocol for `{channel}` already terminated step {step}")
    })?;
    if expected != op {
        return Err(format!(
            "session violation on `{}`: expected {:?} step {}, got {:?}",
            channel, expected, step, op
        ));
    }
    Ok(())
}

fn validate_session_specs() -> Result<Vec<(String, SessionSpec)>, String> {
    let attrs = crate::feature_attrs::find_kind("session");
    let mut out = Vec::with_capacity(attrs.len());
    let mut seen = HashMap::new();

    for (item, rec) in attrs {
        if let Some(previous_line) = seen.insert(item.clone(), rec.line) {
            return Err(diag(
                rec.line,
                format!(
                    "duplicate `#[session]` on `{item}` (first seen at line {})",
                    previous_line.saturating_add(1)
                ),
            ));
        }
        out.push(validate_session_record(&item, &rec)?);
    }

    Ok(out)
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // Rebuild the registry from scratch on every check so failed
    // validation never leaves stale session specs behind.
    install(Vec::new());
    let specs = validate_session_specs()?;
    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn record_session(item: &str, args: &str, line: usize) {
        crate::feature_attrs::record(
            item,
            crate::feature_attrs::AttrRecord {
                name: "session".into(),
                args: args.into(),
                line,
            },
        );
    }

    fn check_ok() {
        let (program, _) = parse("fn main() {}");
        check(&program, "test").expect("session validation should pass");
    }

    fn check_err() -> String {
        let (program, _) = parse("fn main() {}");
        check(&program, "test").expect_err("session validation should fail")
    }

    #[test]
    fn parses_simple_protocol() {
        let p = parse_protocol("send(int).recv(bool).close");
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], SessionOp::Send("int".into()));
        assert_eq!(p[1], SessionOp::Recv("bool".into()));
        assert_eq!(p[2], SessionOp::Close);
    }

    #[test]
    fn validates_in_order() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int).close""#, 0);
        check_ok();
        assert!(validate_step("ch", 0, &SessionOp::Send("int".into())).is_ok());
        assert!(validate_step("ch", 0, &SessionOp::Close).is_err());
        crate::feature_attrs::reset();
    }

    #[test]
    fn accepts_single_close_protocol() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("alpha", r#"protocol = "close""#, 0);
        check_ok();
        assert!(validate_step("alpha", 0, &SessionOp::Close).is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn accepts_send_recv_close_protocol() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("channel", r#"protocol = "send(int).recv(bool).close""#, 0);
        check_ok();
        assert!(validate_step("channel", 0, &SessionOp::Send("int".into())).is_ok());
        assert!(validate_step("channel", 1, &SessionOp::Recv("bool".into())).is_ok());
        assert!(validate_step("channel", 2, &SessionOp::Close).is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn accepts_multiple_session_items() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("left", r#"protocol = "close""#, 0);
        record_session("right", r#"protocol = "send(u8).close""#, 1);
        check_ok();
        assert!(validate_step("left", 0, &SessionOp::Close).is_ok());
        assert!(validate_step("right", 0, &SessionOp::Send("u8".into())).is_ok());
        assert!(validate_step("right", 1, &SessionOp::Close).is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_missing_protocol_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("missing", "", 0);
        let err = check_err();
        assert!(err.contains("missing `protocol = \"...\"`"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_unknown_session_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("unknown", r#"mode = "close""#, 0);
        let err = check_err();
        assert!(
            err.contains("unknown `#[session]` argument `mode`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_unquoted_protocol_value() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("quotes", r#"protocol = close"#, 0);
        let err = check_err();
        assert!(err.contains("must be a quoted string"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_duplicate_protocol_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session(
            "dup-arg",
            r#"protocol = "close", protocol = "send(int).close""#,
            0,
        );
        let err = check_err();
        assert!(err.contains("duplicate `protocol` argument"), "{err}");
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_duplicate_session_declarations() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("dup-item", r#"protocol = "close""#, 0);
        record_session("dup-item", r#"protocol = "send(int).close""#, 1);
        let err = check_err();
        assert!(
            err.contains("duplicate `#[session]` on `dup-item`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn rejects_malformed_protocol_step() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("bad-protocol", r#"protocol = "send(int).bogus.close""#, 0);
        let err = check_err();
        assert!(
            err.contains("invalid session protocol step `bogus`"),
            "{err}"
        );
        crate::feature_attrs::reset();
    }
}
