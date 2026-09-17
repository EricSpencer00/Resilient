//! Integration coverage for RES-3959 (HTTP verify contracts flag).

use serde_json::{Value, json};
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{ServerHandle, send_request_retrying, spawn_with_retry};

const VERIFY_SOURCE: &str = "fn div(int x, int y) -> int requires y != 0 { x / y }";

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

fn verify(server: &ServerHandle, contracts: bool) -> Value {
    let body = json!({
        "tool": "rz_verify",
        "input": {
            "source": VERIFY_SOURCE,
            "contracts": contracts
        }
    })
    .to_string();
    let request = format!(
        "POST /mcp/call HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response = send_request_retrying(
        server.port,
        &request,
        Duration::from_secs(10),
        Duration::from_secs(15),
    )
    .expect("MCP verify request should complete");
    assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
    let (_, response_body) = response
        .split_once("\r\n\r\n")
        .expect("verify response body");
    serde_json::from_str(response_body).expect("verify response JSON")
}

#[test]
fn verify_contracts_flag_returns_proof_status_and_output() {
    let server = spawn_server();

    let skipped = verify(&server, false);
    assert_eq!(skipped["status"], "ok", "got: {skipped}");
    assert_eq!(skipped["proof_status"], "skipped", "got: {skipped}");
    assert!(
        skipped["stdout"]
            .as_str()
            .unwrap_or("")
            .contains("contracts=false")
    );

    let checked = verify(&server, true);
    if cfg!(feature = "z3") {
        assert_eq!(checked["status"], "ok", "got: {checked}");
        assert_eq!(checked["proof_status"], "proved", "got: {checked}");
    } else {
        assert_eq!(checked["status"], "error", "got: {checked}");
        assert_eq!(checked["proof_status"], "unavailable", "got: {checked}");
        assert!(
            checked["stderr"]
                .as_str()
                .unwrap_or("")
                .contains("not available")
        );
    }
}
