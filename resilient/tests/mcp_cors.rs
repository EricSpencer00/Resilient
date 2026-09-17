//! Integration coverage for RES-3962 (configurable CORS and preflight).

use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{ServerHandle, send_request_retrying, spawn_with_retry};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn spawn_server(origin: Option<&str>) -> ServerHandle {
    spawn_with_retry(|port| {
        let mut command = Command::new(bin());
        command
            .arg("mcp")
            .arg("--http-port")
            .arg(format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(origin) = origin {
            command.env("RESILIENT_MCP_CORS_ORIGIN", origin);
        }
        command.spawn()
    })
    .expect("MCP HTTP server should start")
}

fn response(server: &ServerHandle, request: &str) -> String {
    send_request_retrying(
        server.port,
        request,
        Duration::from_secs(5),
        Duration::from_secs(10),
    )
    .expect("MCP HTTP request should complete")
}

fn header<'a>(response: &'a str, name: &str) -> &'a str {
    response
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
        .unwrap_or_else(|| panic!("response is missing {name}:\n{response}"))
}

#[test]
fn cors_headers_are_present_by_default() {
    let server = spawn_server(None);
    let health = response(
        &server,
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    );

    assert!(health.starts_with("HTTP/1.1 200 OK"), "got: {health}");
    assert_eq!(header(&health, "Access-Control-Allow-Origin"), "*");
    assert_eq!(
        header(&health, "Access-Control-Allow-Methods"),
        "GET, POST, OPTIONS"
    );
    assert_eq!(
        header(&health, "Access-Control-Allow-Headers"),
        "Content-Type"
    );
}

#[test]
fn configured_origin_and_preflight_are_supported() {
    let server = spawn_server(Some("https://playground.example"));
    let preflight = response(
        &server,
        "OPTIONS /mcp/call HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://playground.example\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: content-type\r\nConnection: close\r\n\r\n",
    );

    assert!(
        preflight.starts_with("HTTP/1.1 204 No Content"),
        "got: {preflight}"
    );
    assert_eq!(
        header(&preflight, "Access-Control-Allow-Origin"),
        "https://playground.example"
    );
    assert_eq!(header(&preflight, "Content-Length"), "0");
}
