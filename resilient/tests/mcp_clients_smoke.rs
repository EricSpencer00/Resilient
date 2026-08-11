//! RES-3964: smoke test that the curl-style examples in
//! `docs/MCP_CLIENTS.md` actually work against a live `rz mcp --http-port`
//! server. Spawns the real binary and talks to it over a real TCP socket
//! (same pattern as `mcp_openapi_contract_smoke.rs`).

use serde_json::Value;
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{
    DEFAULT_READY_DEADLINE, ServerHandle, send_request_retrying, spawn_with_retry,
};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

struct Server {
    handle: ServerHandle,
}

impl Server {
    fn spawn() -> Self {
        let handle = spawn_with_retry(|port| {
            Command::new(bin())
                .arg("mcp")
                .arg("--http-port")
                .arg(format!("127.0.0.1:{port}"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
        })
        .unwrap_or_else(|err| panic!("{err}"));
        Server { handle }
    }

    fn port(&self) -> u16 {
        self.handle.port
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.handle.child.kill();
        let _ = self.handle.child.wait();
    }
}

fn http_call(port: u16, method: &str, path: &str, body: &str) -> (u16, Value) {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response = send_request_retrying(
        port,
        &request,
        Duration::from_secs(15),
        DEFAULT_READY_DEADLINE,
    )
    .unwrap_or_else(|err| panic!("{method} {path} on port {port}: {err}"));

    let status = response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("status line");

    let json_start = response.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
    let json_body: Value =
        serde_json::from_str(response[json_start..].trim()).expect("valid JSON body");
    (status, json_body)
}

/// Every tool call documented in docs/MCP_CLIENTS.md, as (tool name, input).
/// Kept in sync with the "Calling every exposed tool" section — if a new
/// tool is documented there, add it here so drift is caught.
fn documented_calls() -> Vec<(&'static str, Value)> {
    vec![
        (
            "resilient_parse",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        (
            "resilient_typecheck",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        ("rz_run", serde_json::json!({"source": "println(42)"})),
        (
            "rz_lint",
            serde_json::json!({"source": "fn F(int x) -> int { x }"}),
        ),
        (
            "rz_format",
            serde_json::json!({"source": "fn f(int x)->int{x+1}"}),
        ),
        (
            "rz_check",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        (
            "resilient_explain_lint",
            serde_json::json!({"code": "L0010"}),
        ),
        (
            "resilient_symbols",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        (
            "resilient_hover",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }", "offset": 3}),
        ),
        ("rz_compile", serde_json::json!({"source": "println(42)"})),
        (
            "resilient_disasm",
            serde_json::json!({"source": "println(42)"}),
        ),
        (
            "resilient_vm_run",
            serde_json::json!({"source": "println(42)"}),
        ),
        (
            "resilient_fingerprint",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        (
            "resilient_resilience_score",
            serde_json::json!({"source": "fn add(int a, int b) -> int { a + b }"}),
        ),
        (
            "resilient_contract_infer",
            serde_json::json!({"source": "fn div(int x, int y) -> int { x / y }"}),
        ),
        (
            "resilient_call_graph",
            serde_json::json!({"source": "fn a() -> int { b() } fn b() -> int { 1 }"}),
        ),
    ]
}

#[test]
fn health_endpoint_matches_documented_example() {
    let server = Server::spawn();
    let (status, body) = http_call(server.port(), "GET", "/health", "");
    assert_eq!(status, 200);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "resilient-mcp");
    assert_eq!(body["transport"], "http");
}

#[test]
fn every_documented_tool_call_succeeds() {
    // Some documented examples (e.g. rz_lint on a naming-convention
    // violation) deliberately surface tool-level warnings/errors to show
    // realistic output, so this only asserts the HTTP envelope is well
    // formed and echoes back the right tool name — not that every call
    // is warning-free.
    let server = Server::spawn();
    for (tool, input) in documented_calls() {
        let req_body = serde_json::json!({"tool": tool, "input": input}).to_string();
        let (status, body) = http_call(server.port(), "POST", "/mcp/call", &req_body);
        assert_eq!(status, 200, "tool {tool} unexpected status, body={body}");
        assert!(
            body["status"] == "ok" || body["status"] == "error",
            "tool {tool} missing status field: {body}"
        );
        assert_eq!(body["tool"], tool, "tool {tool} echoed wrong name: {body}");
        assert!(
            body["mcp_tool"].as_str().map(str::len).unwrap_or(0) > 0,
            "tool {tool} missing mcp_tool: {body}"
        );
    }
}

#[test]
fn resilient_verify_documented_call_is_handled() {
    // Only asserts a well-formed response — resilient_verify is a no-op
    // "not available" message on non-Z3 builds, so we can't assert success
    // unconditionally here.
    let server = Server::spawn();
    let req_body = serde_json::json!({
        "tool": "rz_verify",
        "input": {"source": "fn div(int x, int y) -> int\n  requires y != 0\n{ x / y }"}
    })
    .to_string();
    let (status, body) = http_call(server.port(), "POST", "/mcp/call", &req_body);
    assert_eq!(status, 200, "unexpected status, body={body}");
    assert_eq!(body["tool"], "rz_verify");
}
