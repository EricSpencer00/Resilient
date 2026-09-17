//! Integration coverage for RES-3963 (liveness/readiness split).

use serde_json::Value;
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

fn request(server: &ServerHandle, method: &str, path: &str) -> String {
    let request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    send_request_retrying(
        server.port,
        &request,
        Duration::from_secs(5),
        Duration::from_secs(10),
    )
    .expect("MCP HTTP request should complete")
}

fn body(response: &str) -> &str {
    response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .unwrap_or_else(|| panic!("response is missing an HTTP body separator: {response}"))
}

#[test]
fn health_is_liveness_and_readyz_reports_z3_readiness() {
    let server = spawn_server();

    let health = request(&server, "GET", "/health");
    assert!(health.starts_with("HTTP/1.1 200 OK"), "got: {health}");
    let health_json: Value = serde_json::from_str(body(&health)).expect("health JSON");
    assert_eq!(health_json["status"], "ok");

    let ready = request(&server, "GET", "/readyz");
    let ready_json: Value = serde_json::from_str(body(&ready)).expect("readyz JSON");
    if cfg!(feature = "z3") {
        assert!(ready.starts_with("HTTP/1.1 200 OK"), "got: {ready}");
        assert_eq!(ready_json["status"], "ready");
        assert_eq!(ready_json["z3"], "available");
    } else {
        assert!(
            ready.starts_with("HTTP/1.1 503 Service Unavailable"),
            "got: {ready}"
        );
        assert_eq!(ready_json["status"], "not_ready");
        assert_eq!(ready_json["z3"], "unavailable");
        assert!(ready_json["error"].as_str().is_some());
    }

    let preflight = request(&server, "OPTIONS", "/readyz");
    assert!(
        preflight.starts_with("HTTP/1.1 204 No Content"),
        "got: {preflight}"
    );
}
