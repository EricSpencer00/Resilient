//! Integration coverage for RES-3960 opt-in MCP HTTP streaming.

use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{ServerHandle, send_request_retrying, spawn_with_retry};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn spawn_server() -> ServerHandle {
    spawn_with_retry(|port| {
        Command::new(bin())
            .arg("mcp")
            .arg("--http-port")
            .arg(format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })
    .expect("MCP HTTP server should start")
}

#[test]
fn ndjson_accept_opt_in_streams_progress_before_result() {
    let server = spawn_server();
    let body = r#"{"tool":"rz_format","input":{"source":"fn f(int x)->int{x+1}"}}"#;
    let request = format!(
        "POST /v1/mcp/call HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: application/x-ndjson\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let response = send_request_retrying(
        server.port,
        &request,
        Duration::from_secs(5),
        Duration::from_secs(10),
    )
    .expect("streaming MCP HTTP request should complete");

    assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
    assert!(
        response.contains("Content-Type: application/x-ndjson"),
        "got: {response}"
    );
    assert!(
        response.contains("Transfer-Encoding: chunked"),
        "got: {response}"
    );
    let progress = response.find(r#""type":"progress""#);
    let result = response.find(r#""type":"result""#);
    assert!(
        progress.is_some(),
        "stream should emit a progress record: {response}"
    );
    assert!(
        result.is_some(),
        "stream should emit a result record: {response}"
    );
    let progress = progress.unwrap();
    let result = result.unwrap();
    assert!(
        progress < result,
        "progress must precede result: {response}"
    );
    assert!(response.contains(r#""http_status":200"#), "got: {response}");
    assert!(response.contains("fn f(int x) -> int"), "got: {response}");
}
