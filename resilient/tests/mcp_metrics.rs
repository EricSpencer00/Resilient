//! Integration coverage for RES-3952 (Prometheus metrics endpoint).

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

fn metric_value(response: &str, name: &str) -> u64 {
    response
        .lines()
        .find(|line| line.starts_with(name) && line[name.len()..].starts_with(' '))
        .and_then(|line| line.split_whitespace().last())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("response is missing metric {name}:\n{response}"))
}

#[test]
fn metrics_expose_prometheus_counters_and_latency_histogram() {
    let server = spawn_server();

    let missing = request(&server, "GET", "/no-such-route");
    assert!(
        missing.starts_with("HTTP/1.1 404 Not Found"),
        "got: {missing}"
    );

    let metrics = request(&server, "GET", "/metrics");
    assert!(metrics.starts_with("HTTP/1.1 200 OK"), "got: {metrics}");
    assert!(metrics.contains("Content-Type: text/plain; version=0.0.4"));
    assert!(metrics.contains("# TYPE resilient_mcp_http_requests_total counter"));
    assert!(metrics.contains("# TYPE resilient_mcp_http_errors_total counter"));
    assert!(metrics.contains("# TYPE resilient_mcp_http_request_duration_seconds histogram"));
    assert!(metrics.contains("resilient_mcp_http_request_duration_seconds_bucket{le=\"0.001\"}"));
    assert!(metrics.contains("resilient_mcp_http_request_duration_seconds_bucket{le=\"+Inf\"}"));
    assert!(metrics.contains("resilient_mcp_http_request_duration_seconds_sum "));
    assert!(metrics.contains("resilient_mcp_http_request_duration_seconds_count "));

    let requests = metric_value(&metrics, "resilient_mcp_http_requests_total");
    let errors = metric_value(&metrics, "resilient_mcp_http_errors_total");
    let count = metric_value(
        &metrics,
        "resilient_mcp_http_request_duration_seconds_count",
    );
    assert!(
        requests >= 2,
        "expected readiness and route requests: {metrics}"
    );
    assert!(
        errors >= 1,
        "expected the missing route to count as an error: {metrics}"
    );
    assert_eq!(count, requests, "histogram count must match request count");
}
