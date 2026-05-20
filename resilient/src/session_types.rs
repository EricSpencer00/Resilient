//! Feature 20/50 — Session Types.
//!
//! `#[session(protocol = "send(int).recv(bool).close")]` declares a
//! protocol type for a channel: a sequence of operations that must
//! be performed in order. Calls that violate the sequence are
//! rejected at compile time.
//!
//! Protocol grammar (string-encoded for now): operations separated
//! by `.`. Each operation is one of:
//! * `send(T)` — caller sends a value of type T
//! * `recv(T)` — caller receives a value of type T
//! * `close` — terminates the protocol
//!
//! This module records the protocol definitions and exposes a
//! `next_op(channel, operation)` API the runtime / typechecker
//! consults to validate a call.

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

/// RES-2198: dropped the redundant `channel_name: String` field. The
/// only reader was `install`'s key clone (`g.insert(s.channel_name.clone(), s)`),
/// which used it strictly as the HashMap key the spec was stored
/// under. The field stored exactly what the registry key encoded.
/// Pipeline now carries `(String, SessionSpec)` tuples from `collect()`
/// to `install()`. Same dead-field pattern as RES-2106 / … / RES-2196.
#[derive(Debug, Clone)]
pub struct SessionSpec {
    pub protocol: Vec<SessionOp>,
}

static SPECS: LazyLock<RwLock<HashMap<String, SessionSpec>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn parse_protocol(s: &str) -> Vec<SessionOp> {
    // RES-1764: pre-size to (dot-count + 1) — that's the exact number
    // of segments `split('.')` yields. At most one push per segment;
    // sometimes fewer when a segment fails to match any prefix.
    let mut out = Vec::with_capacity(s.matches('.').count() + 1);
    for raw in s.split('.') {
        let op = raw.trim();
        if op == "close" {
            out.push(SessionOp::Close);
        } else if let Some(rest) = op.strip_prefix("send(") {
            if let Some(t) = rest.strip_suffix(')') {
                out.push(SessionOp::Send(t.to_string()));
            }
        } else if let Some(rest) = op.strip_prefix("recv(") {
            if let Some(t) = rest.strip_suffix(')') {
                out.push(SessionOp::Recv(t.to_string()));
            }
        }
    }
    out
}

pub fn collect() -> Vec<(String, SessionSpec)> {
    let attrs = crate::feature_attrs::find_kind("session");
    // RES-1764: pre-size to attrs.len() — exactly one push per
    // attribute record.
    let mut out = Vec::with_capacity(attrs.len());
    for (item, rec) in attrs {
        let mut proto_str = String::new();
        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if let Some((k, v)) = chunk.split_once('=') {
                if k.trim() == "protocol" {
                    proto_str = v.trim().trim_matches('"').to_string();
                }
            }
        }
        out.push((
            item,
            SessionSpec {
                protocol: parse_protocol(&proto_str),
            },
        ));
    }
    out
}

pub fn install(specs: Vec<(String, SessionSpec)>) {
    if let Ok(mut g) = SPECS.write() {
        g.clear();
        // RES-2198: move (channel_name, spec) pairs straight from
        // `collect()` into the map. The previous shape per-spec cloned
        // `s.channel_name` to produce the key, since the field and
        // the key encoded the same string.
        g.extend(specs);
    }
}

pub fn validate_step(channel: &str, step: usize, op: &SessionOp) -> Result<(), String> {
    // RES-1552: hold the read guard so we don't clone the whole
    // `HashMap<String, SessionSpec>` (each spec owns a `Vec<SessionOp>`)
    // just to look up one channel by name. Same lock-then-borrow
    // shape as RES-1544 / RES-1547 / RES-1549.
    let g = SPECS
        .read()
        .map_err(|_| format!("no session protocol for `{channel}`"))?;
    let spec = g
        .get(channel)
        .ok_or_else(|| format!("no session protocol for `{channel}`"))?;
    let expected = spec.protocol.get(step).ok_or_else(|| {
        format!("session protocol for `{channel}` already terminated at step {step}")
    })?;
    if expected != op {
        return Err(format!(
            "session violation on `{}`: expected {:?} at step {}, got {:?}",
            channel, expected, step, op
        ));
    }
    Ok(())
}

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    // RES-1306: gate `install` on the non-empty case — avoids a
    // wasted RwLock write per compilation and removes the
    // wipe-on-empty test race shape documented in RES-1302.
    let specs = collect();
    if specs.is_empty() {
        return Ok(());
    }
    install(specs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        crate::feature_attrs::record(
            "ch",
            crate::feature_attrs::AttrRecord {
                name: "session".into(),
                args: r#"protocol = "send(int).close""#.into(),
                line: 0,
            },
        );
        install(collect());
        assert!(validate_step("ch", 0, &SessionOp::Send("int".into())).is_ok());
        assert!(validate_step("ch", 0, &SessionOp::Close).is_err());
        crate::feature_attrs::reset();
    }
}
