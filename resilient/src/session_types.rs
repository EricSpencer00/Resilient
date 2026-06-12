#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation, dead_code)]

use crate::Node;
use std::collections::{HashMap, HashSet};
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

pub fn parse_protocol(s: &str) -> Vec<SessionOp> {
    let mut out = Vec::new();
    for raw in s.split('.') {
        let op = raw.trim();
        if op == "close" {
            out.push(SessionOp::Close);
        } else if let Some(rest) = op.strip_prefix("send(") {
            if let Some(t) = rest.strip_suffix(')') {
                out.push(SessionOp::Send(t.trim().to_string()));
            }
        } else if let Some(rest) = op.strip_prefix("recv(") {
            if let Some(t) = rest.strip_suffix(')') {
                out.push(SessionOp::Recv(t.trim().to_string()));
            }
        }
    }
    out
}

fn parse_protocol_checked(s: &str) -> Result<Vec<SessionOp>, String> {
    let mut out = Vec::new();
    for raw in s.split('.') {
        let op = raw.trim();
        if op.is_empty() {
            return Err("invalid session protocol: empty operation".to_string());
        }
        if op == "close" {
            out.push(SessionOp::Close);
            continue;
        }

        if let Some(rest) = op.strip_prefix("send(") {
            let Some(t) = rest.strip_suffix(')') else {
                return Err(format!(
                    "invalid session protocol: malformed operation `{op}`"
                ));
            };
            let t = t.trim();
            if t.is_empty() {
                return Err("invalid session protocol: `send` requires type argument".to_string());
            }
            out.push(SessionOp::Send(t.to_string()));
            continue;
        }

        if let Some(rest) = op.strip_prefix("recv(") {
            let Some(t) = rest.strip_suffix(')') else {
                return Err(format!(
                    "invalid session protocol: malformed operation `{op}`"
                ));
            };
            let t = t.trim();
            if t.is_empty() {
                return Err("invalid session protocol: `recv` requires type argument".to_string());
            }
            out.push(SessionOp::Recv(t.to_string()));
            continue;
        }

        return Err(format!(
            "invalid session protocol: unknown operation `{op}`"
        ));
    }
    Ok(out)
}

pub fn collect() -> Vec<(String, SessionSpec)> {
    let attrs = crate::feature_attrs::find_kind("session");
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

fn collect_checked(source_path: &str) -> Result<Vec<(String, SessionSpec)>, String> {
    let attrs = crate::feature_attrs::find_kind("session");
    let mut out = Vec::with_capacity(attrs.len());
    let mut seen_items = HashSet::with_capacity(attrs.len());

    for (item, rec) in attrs {
        if !seen_items.insert(item.clone()) {
            return Err(format!(
                "{source_path}:{}:0: error: duplicate session declaration `{item}`",
                rec.line
            ));
        }

        if rec.args.trim().is_empty() {
            return Err(format!(
                "{source_path}:{}:0: error: session attribute on `{item}` missing `protocol`",
                rec.line
            ));
        }

        let mut proto_str: Option<String> = None;

        for chunk in rec.args.split(',') {
            let chunk = chunk.trim();
            if chunk.is_empty() {
                return Err(format!(
                    "{source_path}:{}:0: error: malformed session attribute on `{item}`: empty argument",
                    rec.line
                ));
            }

            let Some((key, value)) = chunk.split_once('=') else {
                return Err(format!(
                    "{source_path}:{}:0: error: malformed session attribute on `{item}`: expected `key = value`",
                    rec.line
                ));
            };

            let key = key.trim();
            let value = value.trim();
            if key != "protocol" {
                return Err(format!(
                    "{source_path}:{}:0: error: unknown session argument `{key}` on `{item}`",
                    rec.line
                ));
            }

            if proto_str.is_some() {
                return Err(format!(
                    "{source_path}:{}:0: error: duplicate `protocol` argument on `{item}`",
                    rec.line
                ));
            }

            let Some(stripped) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) else {
                return Err(format!(
                    "{source_path}:{}:0: error: session attribute on `{item}` requires quoted `protocol` string",
                    rec.line
                ));
            };
            proto_str = Some(stripped.to_string());
        }

        let Some(proto_str) = proto_str else {
            return Err(format!(
                "{source_path}:{}:0: error: session attribute on `{item}` missing `protocol`",
                rec.line
            ));
        };

        let protocol = parse_protocol_checked(&proto_str).map_err(|msg| {
            format!(
                "{source_path}:{}:0: error: session attribute on `{item}`: {msg}",
                rec.line
            )
        })?;
        out.push((item, SessionSpec { protocol }));
    }

    Ok(out)
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
    let expected = spec
        .protocol
        .get(step)
        .ok_or_else(|| format!("session protocol `{channel}` already terminated step {step}"))?;
    if expected != op {
        return Err(format!(
            "session violation on `{}`: expected {:?} step {}, got {:?}",
            channel, expected, step, op
        ));
    }
    Ok(())
}

pub(crate) fn check(_program: &Node, source_path: &str) -> Result<(), String> {
    let specs = collect_checked(source_path)?;
    if specs.is_empty() {
        return Ok(());
    }
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

    fn dummy_program() -> Node {
        let (prog, errs) = parse("");
        assert!(errs.is_empty(), "unexpected parse errors: {errs:?}");
        prog
    }

    fn run_check(source_path: &str) -> Result<(), String> {
        check(&dummy_program(), source_path)
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

    #[test]
    fn check_accepts_empty_session_registry() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        assert!(run_check("session.rz").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_single_valid_session_protocol() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int).close""#, 11);
        assert!(run_check("session.rz").is_ok());
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_accepts_multiple_valid_session_protocols() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("client", r#"protocol = "send(int).recv(bool).close""#, 21);
        record_session("server", r#"protocol = "recv(int).close""#, 22);
        let result = run_check("session.rz");
        assert!(
            result.is_ok(),
            "expected valid session declarations: {result:?}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_missing_protocol_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", "", 31);
        let err = run_check("session.rz").expect_err("expected error missing protocol");
        assert!(
            err.contains("session.rz:31:0: error:"),
            "missing line/column in diagnostic: {err}"
        );
        assert!(
            err.contains("missing `protocol`"),
            "wrong diagnostic missing protocol: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_protocol_arguments() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session(
            "ch",
            r#"protocol = "send(int).close", protocol = "recv(int).close""#,
            32,
        );
        let err = run_check("session.rz").expect_err("expected error duplicate protocol");
        assert!(
            err.contains("session.rz:32:0: error:"),
            "missing line/column in diagnostic: {err}"
        );
        assert!(
            err.contains("duplicate `protocol`"),
            "wrong diagnostic duplicate protocol argument: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_unknown_session_argument() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int).close", mode = "strict""#, 33);
        let err = run_check("session.rz").expect_err("expected error unknown argument");
        assert!(
            err.contains("session.rz:33:0: error:"),
            "missing line/column in diagnostic: {err}"
        );
        assert!(
            err.contains("unknown session argument `mode`"),
            "wrong diagnostic unknown argument: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_duplicate_session_declarations() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int).close""#, 41);
        record_session("ch", r#"protocol = "recv(int).close""#, 42);
        let err = run_check("session.rz").expect_err("expected error duplicate declarations");
        assert!(
            err.contains("session.rz:42:0: error:"),
            "expected second declaration location in diagnostic: {err}"
        );
        assert!(
            err.contains("duplicate session declaration"),
            "wrong diagnostic duplicate session declaration: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_malformed_protocol_step() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int).bogus.close""#, 51);
        let err = run_check("session.rz").expect_err("expected error malformed protocol");
        assert!(
            err.contains("session.rz:51:0: error:"),
            "missing line/column in diagnostic: {err}"
        );
        assert!(
            err.contains("invalid session protocol"),
            "wrong diagnostic malformed protocol: {err}"
        );
        crate::feature_attrs::reset();
    }

    #[test]
    fn check_rejects_malformed_send_form() {
        let _g = crate::feature_attrs::lock_for_test();
        crate::feature_attrs::reset();
        record_session("ch", r#"protocol = "send(int.close""#, 52);
        let err = run_check("session.rz").expect_err("expected error malformed send");
        assert!(
            err.contains("session.rz:52:0: error:"),
            "missing line/column in diagnostic: {err}"
        );
        assert!(
            err.contains("invalid session protocol"),
            "wrong diagnostic for malformed send: {err}"
        );
        crate::feature_attrs::reset();
    }
}
