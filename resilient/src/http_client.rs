//! RES-2556: minimal HTTP/1.1 client builtins (std-only).
//!
//! Provides `http_get` and `http_post` built on raw TCP sockets.
//! Handles chunked transfer encoding, Content-Length, request
//! headers, response headers, and configurable timeouts. No TLS —
//! HTTP only.

#![allow(clippy::collapsible_if, clippy::doc_lazy_continuation)]

use crate::{MapKey, Node, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

type RResult<T> = Result<T, String>;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

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

#[derive(Debug)]
struct RequestOptions {
    headers: HashMap<String, String>,
    timeout: Duration,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            headers: HashMap::new(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }
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

fn response_headers_to_map(headers: Vec<(String, String)>) -> Value {
    let mut map = HashMap::with_capacity(headers.len());
    for (key, value) in headers {
        map.insert(MapKey::Str(key), Value::String(value));
    }
    Value::Map(map)
}

fn make_response(status: i64, body: String, headers: Vec<(String, String)>) -> Value {
    Value::Struct {
        name: "Response".to_string(),
        fields: vec![
            ("status".to_string(), Value::Int(status)),
            ("body".to_string(), Value::String(body)),
            ("headers".to_string(), response_headers_to_map(headers)),
        ],
    }
}

fn timeout_from_ms(ms: i64, builtin: &str) -> RResult<Duration> {
    if ms <= 0 {
        return Err(format!(
            "{builtin}: timeout must be a positive integer number of milliseconds"
        ));
    }
    Ok(Duration::from_millis(ms as u64))
}

fn request_headers_from_map(
    headers: &HashMap<MapKey, Value>,
    builtin: &str,
) -> RResult<HashMap<String, String>> {
    let mut out = HashMap::with_capacity(headers.len());
    for (key, value) in headers {
        let key = match key {
            MapKey::Str(s) => s.clone(),
            other => {
                return Err(format!(
                    "{builtin}: header names must be strings, got {}",
                    other
                ));
            }
        };
        let val = match value {
            Value::String(s) => s.clone(),
            other => {
                return Err(format!(
                    "{builtin}: header `{}` must be a string value, got {}",
                    key, other
                ));
            }
        };
        out.insert(key, val);
    }
    Ok(out)
}

fn parse_request_options(args: &[Value], start: usize, builtin: &str) -> RResult<RequestOptions> {
    let mut options = RequestOptions::default();
    let mut saw_headers = false;
    let mut saw_timeout = false;
    for arg in &args[start..] {
        match arg {
            Value::Map(map) => {
                if saw_headers {
                    return Err(format!(
                        "{builtin}: request headers specified more than once"
                    ));
                }
                options.headers = request_headers_from_map(map, builtin)?;
                saw_headers = true;
            }
            Value::Int(ms) => {
                if saw_timeout {
                    return Err(format!("{builtin}: timeout specified more than once"));
                }
                options.timeout = timeout_from_ms(*ms, builtin)?;
                saw_timeout = true;
            }
            other => {
                return Err(format!(
                    "{builtin}: expected an optional headers map or timeout integer, got {}",
                    other
                ));
            }
        }
    }
    Ok(options)
}

fn connect_with_timeout(parsed: &ParsedUrl, timeout: Duration) -> Result<TcpStream, String> {
    let addr = format!("{}:{}", parsed.host, parsed.port);
    let mut last_err: Option<String> = None;
    let addrs = addr
        .to_socket_addrs()
        .map_err(|e| format!("connection failed: could not resolve {}: {}", addr, e))?;
    for socket_addr in addrs {
        match TcpStream::connect_timeout(&socket_addr, timeout) {
            Ok(stream) => return Ok(stream),
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    Err(match last_err {
        Some(e) => format!("connection failed: {}", e),
        None => format!(
            "connection failed: no socket addresses resolved for {}",
            addr
        ),
    })
}

fn host_header(parsed: &ParsedUrl) -> String {
    if parsed.port == 80 {
        parsed.host.clone()
    } else {
        format!("{}:{}", parsed.host, parsed.port)
    }
}

fn send_request(method: &str, url: &str, body: Option<&str>, options: RequestOptions) -> Value {
    let parsed = match parse_url(url) {
        Ok(p) => p,
        Err(e) => return err_val(e),
    };

    let mut stream = match connect_with_timeout(&parsed, options.timeout) {
        Ok(s) => s,
        Err(e) => return err_val(e),
    };

    if stream
        .set_read_timeout(Some(options.timeout))
        .and_then(|_| stream.set_write_timeout(Some(options.timeout)))
        .is_err()
    {
        return err_val("failed to set socket timeout".to_string());
    }

    let mut headers = HashMap::with_capacity(options.headers.len() + 4);
    headers.insert("User-Agent".to_string(), "Resilient/1.0".to_string());
    for (k, v) in options.headers {
        headers.insert(k, v);
    }
    headers.insert("Host".to_string(), host_header(&parsed));
    headers.insert("Connection".to_string(), "close".to_string());
    if let Some(b) = body {
        headers.insert("Content-Length".to_string(), b.len().to_string());
        headers
            .entry("Content-Type".to_string())
            .or_insert_with(|| "application/json".to_string());
    }

    let mut request = format!("{} {} HTTP/1.1\r\n", method, parsed.path);
    for (k, v) in headers {
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
    if args.is_empty() {
        return Err("http_get: expected at least 1 argument, got 0".to_string());
    }
    let url = match &args[0] {
        Value::String(url) => url,
        other => return Err(format!("http_get: expected string URL, got {}", other)),
    };
    if args.len() > 3 {
        return Err(format!(
            "http_get: expected 1 to 3 arguments, got {}",
            args.len()
        ));
    }
    let options = parse_request_options(args, 1, "http_get")?;
    Ok(send_request("GET", url, None, options))
}

pub(crate) fn builtin_http_post(args: &[Value]) -> RResult<Value> {
    if args.len() < 2 {
        return Err(format!(
            "http_post: expected at least 2 arguments, got {}",
            args.len()
        ));
    }
    let url = match &args[0] {
        Value::String(url) => url,
        other => return Err(format!("http_post: expected string URL, got {}", other)),
    };
    let body = match &args[1] {
        Value::String(body) => body,
        other => return Err(format!("http_post: expected string body, got {}", other)),
    };
    if args.len() > 4 {
        return Err(format!(
            "http_post: expected 2 to 4 arguments, got {}",
            args.len()
        ));
    }
    let options = parse_request_options(args, 2, "http_post")?;
    Ok(send_request("POST", url, Some(body), options))
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
    use crate::typechecker::TypeChecker;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    fn s(v: &str) -> Value {
        Value::String(v.to_string())
    }

    fn typechecks(src: &str) {
        let (prog, errs) = crate::parse(src);
        assert!(errs.is_empty(), "parse errors: {:?}", errs);
        TypeChecker::new()
            .check_program(&prog)
            .expect("program should typecheck");
    }

    fn headers_map(pairs: &[(&str, &str)]) -> Value {
        let mut map = HashMap::with_capacity(pairs.len());
        for (k, v) in pairs {
            map.insert(
                MapKey::Str((*k).to_string()),
                Value::String((*v).to_string()),
            );
        }
        Value::Map(map)
    }

    fn read_request_text(stream: &mut std::net::TcpStream, needle: &str) -> String {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buf);
            if text.contains(needle) {
                break;
            }
        }
        String::from_utf8(buf).unwrap()
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
                Value::Struct {
                    ref name,
                    ref fields,
                    ..
                } => {
                    assert_eq!(name, "Response");
                    let status = fields.iter().find(|(k, _)| k == "status");
                    assert!(matches!(status, Some((_, Value::Int(200)))));
                    let body = fields.iter().find(|(k, _)| k == "body");
                    match body {
                        Some((_, Value::String(s))) => assert_eq!(s, "hello world"),
                        _ => panic!("expected body string"),
                    }
                    let headers = fields.iter().find(|(k, _)| k == "headers");
                    match headers {
                        Some((_, Value::Map(headers))) => {
                            assert!(matches!(
                                headers.get(&MapKey::Str("content-type".to_string())),
                                Some(Value::String(s)) if s == "text/plain"
                            ));
                        }
                        _ => panic!("expected headers map"),
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
                Value::Struct {
                    ref name,
                    ref fields,
                    ..
                } => {
                    assert_eq!(name, "Response");
                    let body = fields.iter().find(|(k, _)| k == "body");
                    match body {
                        Some((_, Value::String(s))) => assert_eq!(s, "hello"),
                        _ => panic!("expected body string"),
                    }
                    let headers = fields.iter().find(|(k, _)| k == "headers");
                    match headers {
                        Some((_, Value::Map(headers))) => {
                            assert!(matches!(
                                headers.get(&MapKey::Str("transfer-encoding".to_string())),
                                Some(Value::String(s)) if s == "chunked"
                            ));
                        }
                        _ => panic!("expected headers map"),
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
    fn http_get_round_trip_supports_headers_and_response_map() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_text(&mut stream, "\r\n\r\n");
            assert!(request.contains("GET /data HTTP/1.1"));
            assert!(request.contains("X-Test: one"));
            assert!(request.contains("Connection: close"));
            let response = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nX-Trace: abc\r\nContent-Length: 5\r\n\r\nhello";
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let url = format!("http://127.0.0.1:{}/data", port);
        let result = builtin_http_get(&[s(&url), headers_map(&[("X-Test", "one")])]).unwrap();
        match result {
            Value::Result { ok: true, payload } => match *payload {
                Value::Struct { name, fields } => {
                    assert_eq!(name, "Response");
                    let body = fields.iter().find(|(k, _)| k == "body");
                    match body {
                        Some((_, Value::String(s))) => assert_eq!(s, "hello"),
                        _ => panic!("expected body string"),
                    }
                    let headers = fields.iter().find(|(k, _)| k == "headers");
                    match headers {
                        Some((_, Value::Map(map))) => {
                            assert!(matches!(
                                map.get(&MapKey::Str("content-type".to_string())),
                                Some(Value::String(s)) if s == "text/plain"
                            ));
                            assert!(matches!(
                                map.get(&MapKey::Str("x-trace".to_string())),
                                Some(Value::String(s)) if s == "abc"
                            ));
                        }
                        _ => panic!("expected response headers map"),
                    }
                }
                other => panic!("expected response struct, got {:?}", other),
            },
            other => panic!("expected Ok result, got {:?}", other),
        }
        server.join().unwrap();
    }

    #[test]
    fn http_post_round_trip_supports_body_headers_and_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request_text(&mut stream, "{\"ping\":true}");
            assert!(request.contains("POST /submit HTTP/1.1"));
            assert!(request.contains("Content-Type: application/json"));
            assert!(request.contains("X-Token: abc123"));
            assert!(request.contains("{\"ping\":true}"));
            let response = "HTTP/1.1 201 Created\r\nX-Reply: yes\r\nContent-Length: 2\r\n\r\nok";
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
        });

        let url = format!("http://127.0.0.1:{}/submit", port);
        let result = builtin_http_post(&[
            s(&url),
            s("{\"ping\":true}"),
            headers_map(&[("X-Token", "abc123")]),
            Value::Int(250),
        ])
        .unwrap();
        match result {
            Value::Result { ok: true, payload } => match *payload {
                Value::Struct { name, fields } => {
                    assert_eq!(name, "Response");
                    let status = fields.iter().find(|(k, _)| k == "status");
                    assert!(matches!(status, Some((_, Value::Int(201)))));
                    let headers = fields.iter().find(|(k, _)| k == "headers");
                    match headers {
                        Some((_, Value::Map(map))) => {
                            assert!(matches!(
                                map.get(&MapKey::Str("x-reply".to_string())),
                                Some(Value::String(s)) if s == "yes"
                            ));
                        }
                        _ => panic!("expected response headers map"),
                    }
                }
                other => panic!("expected response struct, got {:?}", other),
            },
            other => panic!("expected Ok result, got {:?}", other),
        }
        server.join().unwrap();
    }

    #[test]
    fn http_get_honors_timeout_parameter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _request = read_request_text(&mut stream, "\r\n\r\n");
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = stream.shutdown(std::net::Shutdown::Both);
        });

        let url = format!("http://127.0.0.1:{}/slow", port);
        let start = Instant::now();
        let result = builtin_http_get(&[s(&url), Value::Int(50)]).unwrap();
        let elapsed = start.elapsed();
        assert!(matches!(result, Value::Result { ok: false, .. }));
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "timeout should fire quickly, elapsed: {:?}",
            elapsed
        );
        server.join().unwrap();
    }

    #[test]
    fn http_get_accepts_optional_headers_and_timeout_in_typechecker() {
        typechecks(
            r#"
let headers = {"X-Test" -> "one"};
let resp = http_get("http://example.com/data", headers, 50);
"#,
        );
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
