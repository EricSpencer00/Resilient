//! RES-3961: contract tests asserting live MCP HTTP wrapper responses
//! conform to the OpenAPI document at `docs/openapi.json`.
//!
//! These spawn the real `rz` binary and talk to it over a real TCP
//! socket (same pattern as `mcp_http_batch2_smoke.rs`), then check the
//! response shape against the hand-written schemas rather than parsing
//! the OpenAPI file itself — the goal is to catch drift between the doc
//! and the implementation, not to build a general JSON-schema validator.
//!
//! RES-4204: startup readiness and every request go through the shared
//! retry helpers in `mcp_smoke_support` instead of a single attempt, to
//! absorb both the CI startup race between spawning `rz` and its listener
//! accepting connections, and the narrower race where a connection is
//! accepted but reset before a response is written under CI contention.

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;

use serde_json::Value;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn spawn() -> Self {
        let port = free_port();
        let child = Command::new(bin())
            .arg("mcp")
            .arg("--http-port")
            .arg(format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn rz mcp --http-port");
        let server = Server { child, port };
        if let Err(err) = mcp_smoke_support::wait_for_health(
            server.port,
            mcp_smoke_support::DEFAULT_READY_DEADLINE,
        ) {
            panic!("{err}");
        }
        server
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn http_call(port: u16, method: &str, path: &str, body: &str) -> (u16, Value) {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response = match mcp_smoke_support::send_request_retrying(
        port,
        &request,
        Duration::from_secs(15),
        mcp_smoke_support::DEFAULT_READY_DEADLINE,
    ) {
        Ok(response) => response,
        Err(err) => panic!("{err}"),
    };

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

fn openapi_doc() -> Value {
    let raw = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/openapi.json"))
        .expect("docs/openapi.json must exist");
    serde_json::from_str(&raw).expect("docs/openapi.json must be valid JSON")
}

#[test]
fn openapi_doc_describes_health_and_mcp_call() {
    let doc = openapi_doc();
    assert_eq!(doc["openapi"], "3.0.3");
    assert!(doc["paths"]["/health"]["get"].is_object());
    assert!(doc["paths"]["/mcp/call"]["post"].is_object());
    for status in ["200", "400", "404", "413", "429", "504"] {
        assert!(
            doc["paths"]["/mcp/call"]["post"]["responses"][status].is_object(),
            "openapi.json missing /mcp/call response for status {status}"
        );
    }
}

#[test]
fn health_response_matches_schema() {
    let server = Server::spawn();
    let (status, body) = http_call(server.port, "GET", "/health", "");
    assert_eq!(status, 200);

    let doc = openapi_doc();
    let required = doc["components"]["schemas"]["HealthResponse"]["required"]
        .as_array()
        .unwrap();
    for field in required {
        let field = field.as_str().unwrap();
        assert!(
            body.get(field).is_some(),
            "/health response missing schema-required field `{field}`: {body}"
        );
    }
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], "resilient-mcp");
    assert_eq!(body["transport"], "http");
}

#[test]
fn mcp_call_success_response_matches_schema() {
    let server = Server::spawn();
    let req_body = serde_json::json!({
        "tool": "rz_run",
        "input": { "source": "println(42)" }
    })
    .to_string();
    let (status, body) = http_call(server.port, "POST", "/mcp/call", &req_body);
    assert_eq!(status, 200, "unexpected status, body={body}");

    let doc = openapi_doc();
    let required = doc["components"]["schemas"]["McpCallResponse"]["required"]
        .as_array()
        .unwrap();
    for field in required {
        let field = field.as_str().unwrap();
        assert!(
            body.get(field).is_some(),
            "/mcp/call response missing schema-required field `{field}`: {body}"
        );
    }
    assert_eq!(body["status"], "ok");
    assert_eq!(body["tool"], "rz_run");
    assert_eq!(body["mcp_tool"], "resilient_run");
}

#[test]
fn mcp_call_missing_tool_matches_error_schema() {
    let server = Server::spawn();
    let (status, body) = http_call(server.port, "POST", "/mcp/call", "{}");
    assert_eq!(status, 400);

    let doc = openapi_doc();
    let required = doc["components"]["schemas"]["ErrorResponse"]["required"]
        .as_array()
        .unwrap();
    for field in required {
        let field = field.as_str().unwrap();
        assert!(
            body.get(field).is_some(),
            "error response missing schema-required field `{field}`: {body}"
        );
    }
    assert_eq!(body["status"], "error");
}

#[test]
fn unsupported_route_matches_error_schema_and_404() {
    let server = Server::spawn();
    let (status, body) = http_call(server.port, "GET", "/no-such-route", "");
    assert_eq!(status, 404);
    assert_eq!(body["status"], "error");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("unsupported route")
    );
}
