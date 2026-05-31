//! RES-2556: minimal HTTP/1.1 client builtins (std-only).
//!
//! Provides `http_get` and `http_post` built on raw TCP sockets.
//! Handles chunked transfer encoding, Content-Length, and header
//! parsing. No TLS — HTTP only.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::{Node, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

type RResult<T> = Result<T, String>;

fn ok(v: Value) -> Value {
    Value::Result {
        ok: true,
        payload: Box::new(v),
    }
}

fn err_val(msg: String) -> Value {
    Value::Result {
        ok: false,
        payload: Box::new(Value::String(msg)),
    }
}

struct ParsedUrl {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<ParsedUrl, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("http_*: only http:// URLs supported, got: {url}"))?;

    let (host_port, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };

    let (host, port) = match host_port.rfind(':') {
        Some(i) => {
            let port_str = &host_port[i + 1..];
            let port: u16 = port_str
                .parse()
                .map_err(|_| format!("http_*: invalid port: {port_str}"))?;
            (&host_port[..i], port)
        }
        None => (host_port, 80),
    };

    if host.is_empty() {
        return Err("http_*: empty host".to_string());
    }

    Ok(ParsedUrl {
        host: host.to_string(),
        port,
        path: path.to_string(),
    })
}

fn make_response(status: i64, body: String, headers: Vec<(String, String)>) -> Value {
    let header_values: Vec<Value> = headers
        .into_iter()
        .map(|(k, v)| Value::Tuple(vec![Value::String(k), Value::String(v)]))
        .collect();
    Value::Struct {
        name: "HttpResponse".to_string(),
        fields: vec![
            ("status".to_string(), Value::Int(status)),
            ("body".to_string(), Value::String(body)),
            ("headers".to_string(), Value::Array(header_values)),
        ],
    }
}

fn send_request(
    method: &str,
    url: &str,
    body: Option<&str>,
    extra_headers: &[(String, String)],
) -> Value {
    let parsed = match parse_url(url) {
        Ok(p) => p,
        Err(e) => return err_val(e),
    };

    let mut stream = match TcpStream::connect(format!("{}:{}", parsed.host, parsed.port)) {
        Ok(s) => s,
        Err(e) => return err_val(format!("connection failed: {e}")),
    };

    if stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .is_err()
    {
        return err_val("failed to set read timeout".to_string());
    }

    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
        method, parsed.path, parsed.host
    );

    if let Some(b) = body {
        request.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }

    for (k, v) in extra_headers {
        request.push_str(&format!("{}: {}\r\n", k, v));
    }

    request.push_str("\r\n");

    if let Some(b) = body {
        request.push_str(b);
    }

    if let Err(e) = stream.write_all(request.as_bytes()) {
        return err_val(format!("write failed: {e}"));
    }
    if let Err(e) = stream.flush() {
        return err_val(format!("flush failed: {e}"));
    }

    let mut raw = Vec::new();
    if let Err(e) = stream.read_to_end(&mut raw) {
        return err_val(format!("read failed: {e}"));
    }

    let raw_str = String::from_utf8_lossy(&raw);
    parse_http_response(&raw_str)
}

fn parse_http_response(raw: &str) -> Value {
    let header_end = match raw.find("\r\n\r\n") {
        Some(i) => i,
        None => return err_val("malformed HTTP response: no header/body separator".to_string()),
    };

    let header_section = &raw[..header_end];
    let body_raw = &raw[header_end + 4..];

    let mut lines = header_section.split("\r\n");
    let status_line = match lines.next() {
        Some(l) => l,
        None => return err_val("empty HTTP response".to_string()),
    };

    let status = parse_status_code(status_line);

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut is_chunked = false;
    for line in lines {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let val = line[colon + 1..].trim().to_string();
            if key == "transfer-encoding" && val.to_lowercase().contains("chunked") {
                is_chunked = true;
            }
            headers.push((key, val));
        }
    }

    let body = if is_chunked {
        decode_chunked(body_raw)
    } else {
        body_raw.to_string()
    };

    ok(make_response(status, body, headers))
}

fn parse_status_code(status_line: &str) -> i64 {
    let parts: Vec<&str> = status_line.splitn(3, ' ').collect();
    if parts.len() >= 2 {
        parts[1].parse().unwrap_or(0)
    } else {
        0
    }
}

fn decode_chunked(data: &str) -> String {
    let mut result = String::new();
    let mut remaining = data;

    while let Some(line_end) = remaining.find("\r\n") {
        let size_str = remaining[..line_end].trim();
        let chunk_size = match i64::from_str_radix(size_str, 16) {
            Ok(s) => s as usize,
            Err(_) => break,
        };
        if chunk_size == 0 {
            break;
        }
        let chunk_start = line_end + 2;
        if chunk_start + chunk_size > remaining.len() {
            result.push_str(&remaining[chunk_start..]);
            break;
        }
        result.push_str(&remaining[chunk_start..chunk_start + chunk_size]);
        remaining = &remaining[chunk_start + chunk_size..];
        if remaining.starts_with("\r\n") {
            remaining = &remaining[2..];
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Builtins
// ---------------------------------------------------------------------------

pub(crate) fn builtin_http_get(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(url)] => Ok(send_request("GET", url, None, &[])),
        [other] => Err(format!("http_get: expected string URL, got {}", other)),
        _ => Err(format!("http_get: expected 1 argument, got {}", args.len())),
    }
}

pub(crate) fn builtin_http_post(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(url), Value::String(body)] => Ok(send_request("POST", url, Some(body), &[])),
        [a, b] => Err(format!(
            "http_post: expected (string, string), got ({}, {})",
            a, b
        )),
        _ => Err(format!(
            "http_post: expected 2 arguments, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Feature pass (no-op)
// ---------------------------------------------------------------------------

pub(crate) fn check(_program: &Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    #[test]
    fn parse_url_basic() {
        let p = parse_url("http://example.com/path").unwrap();
        assert_eq!(p.host, "example.com");
        assert_eq!(p.port, 80);
        assert_eq!(p.path, "/path");
    }

    #[test]
    fn parse_url_with_port() {
        let p = parse_url("http://localhost:8080/api/data").unwrap();
        assert_eq!(p.host, "localhost");
        assert_eq!(p.port, 8080);
        assert_eq!(p.path, "/api/data");
    }

    #[test]
    fn parse_url_no_path() {
        let p = parse_url("http://example.com").unwrap();
        assert_eq!(p.host, "example.com");
        assert_eq!(p.path, "/");
    }

    #[test]
    fn parse_url_rejects_https() {
        assert!(parse_url("https://example.com").is_err());
    }

    #[test]
    fn parse_url_rejects_empty_host() {
        assert!(parse_url("http:///path").is_err());
    }

    #[test]
    fn parse_status_code_200() {
        assert_eq!(parse_status_code("HTTP/1.1 200 OK"), 200);
    }

    #[test]
    fn parse_status_code_404() {
        assert_eq!(parse_status_code("HTTP/1.1 404 Not Found"), 404);
    }

    #[test]
    fn parse_status_code_malformed() {
        assert_eq!(parse_status_code("garbage"), 0);
    }

    #[test]
    fn decode_chunked_basic() {
        let input = "5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(input), "hello world");
    }

    #[test]
    fn decode_chunked_single() {
        let input = "3\r\nfoo\r\n0\r\n\r\n";
        assert_eq!(decode_chunked(input), "foo");
    }

    #[test]
    fn parse_response_basic() {
        let raw = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello world";
        let result = parse_http_response(raw);
        match result {
            Value::Result {
                ok: true, payload, ..
            } => match *payload {
                Value::Struct { ref fields, .. } => {
                    let status = fields.iter().find(|(k, _)| k == "status");
                    assert!(matches!(status, Some((_, Value::Int(200)))));
                    let body = fields.iter().find(|(k, _)| k == "body");
                    match body {
                        Some((_, Value::String(s))) => assert_eq!(s, "hello world"),
                        _ => panic!("expected body string"),
                    }
                }
                _ => panic!("expected struct"),
            },
            _ => panic!("expected Ok result"),
        }
    }

    #[test]
    fn parse_response_chunked() {
        let raw = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let result = parse_http_response(raw);
        match result {
            Value::Result {
                ok: true, payload, ..
            } => match *payload {
                Value::Struct { ref fields, .. } => {
                    let body = fields.iter().find(|(k, _)| k == "body");
                    match body {
                        Some((_, Value::String(s))) => assert_eq!(s, "hello"),
                        _ => panic!("expected body string"),
                    }
                }
                _ => panic!("expected struct"),
            },
            _ => panic!("expected Ok result"),
        }
    }

    #[test]
    fn http_get_wrong_args_error() {
        assert!(builtin_http_get(&[Value::Int(42)]).is_err());
        assert!(builtin_http_get(&[s("a"), s("b")]).is_err());
    }

    #[test]
    fn http_post_wrong_args_error() {
        assert!(builtin_http_post(&[s("url")]).is_err());
        assert!(builtin_http_post(&[Value::Int(1), s("body")]).is_err());
    }

    #[test]
    fn http_get_bad_url_returns_err() {
        let result = builtin_http_get(&[s("ftp://example.com")]).unwrap();
        assert!(matches!(result, Value::Result { ok: false, .. }));
    }

    #[test]
    fn http_get_connection_refused() {
        let result = builtin_http_get(&[s("http://127.0.0.1:1/test")]).unwrap();
        assert!(matches!(result, Value::Result { ok: false, .. }));
    }

    #[test]
    fn end_to_end_http_get_bad_url() {
        let r = crate::run_program(
            r#"
let result = http_get("ftp://bad")
match result {
    Ok(r) => println("unexpected ok"),
    Err(e) => println("error: " + e),
}
"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("error:"));
    }
}
