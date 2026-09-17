//! Integration coverage for RES-3966 (HTTP bind failure fallback).

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{ServerHandle, spawn_with_retry};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn spawn_http_server() -> ServerHandle {
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

fn spawn_conflicting_server(port: u16) -> std::io::Result<Child> {
    Command::new(bin())
        .arg("mcp")
        .arg("--http-port")
        .arg(format!("127.0.0.1:{port}"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

#[test]
fn occupied_http_port_falls_back_to_stdio_mcp() {
    let mut occupied = spawn_http_server();
    let mut fallback = spawn_conflicting_server(occupied.port).expect("spawn fallback process");

    let request = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}
"#;
    fallback
        .stdin
        .as_mut()
        .expect("fallback stdin")
        .write_all(request.as_bytes())
        .expect("write MCP request to fallback");

    let mut output = String::new();
    BufReader::new(fallback.stdout.take().expect("fallback stdout"))
        .read_line(&mut output)
        .expect("read MCP response from fallback");
    let response: Value = serde_json::from_str(output.trim()).expect("fallback MCP JSON response");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(response["result"].is_object(), "got: {response}");

    let _ = fallback.kill();
    let _ = fallback.wait();
    let _ = occupied.child.kill();
    let _ = occupied.child.wait();
}
