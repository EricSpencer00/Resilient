//! RES-3967: enforce the Live MCP Server initiative's <2s per-request
//! latency SLA (#3934) with a real end-to-end measurement.
//!
//! Spawns the real `rz mcp --http-port` binary, warms it up, then sends
//! N repeated requests for each of a representative set of tool calls
//! (`/health`, `rz_parse`, `rz_typecheck`, `rz_run`) and asserts that the
//! median (p50) wall-clock latency stays comfortably under the 2s SLA.
//!
//! Per the perf-gate lesson from PR #4120: a single sample of a
//! sub-second operation on a noisy/shared hosted runner is not a
//! reliable signal — one slow scheduling tick can blow past any tight
//! bound and flake the suite. We take N=20 samples per call and assert
//! against the *median*, which is robust to a handful of outliers, and
//! we leave a large margin under the 2s SLA (asserting < 2s directly,
//! not some tighter number) so this test only fails if the SLA itself
//! would actually be violated for a typical request, not because CI was
//! briefly under load.
//!
//! This is a smoke test, not a micro-benchmark: for a stable statistical
//! report (p95, min/max, per-tool breakdown) across many warmup/sample
//! counts, use `benchmarks/mcp_latency/run.sh` instead.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "mcp_smoke_support/mod.rs"]
mod mcp_smoke_support;
use mcp_smoke_support::{DEFAULT_READY_DEADLINE, send_request_retrying, wait_for_health};

/// The SLA this test enforces (matches the Live MCP Server initiative's
/// stated <2s success metric, #3934).
const SLA: Duration = Duration::from_secs(2);

/// Samples per tool call. Enough to make the median robust to a couple
/// of noisy outliers without making the test slow (worst case: this
/// many requests times a handful of tools times a fraction of a second
/// each, still well under a minute even on a slow runner).
const SAMPLES: usize = 20;

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
        wait_for_health(port, DEFAULT_READY_DEADLINE)
            .unwrap_or_else(|err| panic!("server on port {} never became healthy: {err}", port));
        server
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn http_call(port: u16, method: &str, path: &str, body: &str) -> Result<String, String> {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    send_request_retrying(
        port,
        &request,
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
}

fn status_of(response: &str) -> &str {
    response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?")
}

/// A small representative Resilient program: enough surface for parse,
/// typecheck, and run tool calls to do real work without turning this
/// into a workload benchmark.
const SAMPLE_SOURCE: &str = r#"
fn add(a: int, b: int) -> int {
    return a + b;
}
fn main() -> int {
    let total: int = 0;
    let i: int = 0;
    while i < 50 {
        total = add(total, i);
        i = i + 1;
    }
    return total;
}
main();
"#;

fn tool_call_body(tool: &str) -> String {
    serde_json::json!({
        "tool": tool,
        "input": { "source": SAMPLE_SOURCE }
    })
    .to_string()
}

/// Send `request_fn` `SAMPLES` times against `port`, returning per-call
/// wall-clock durations. Panics immediately on any non-2xx response —
/// latency numbers are meaningless if the calls themselves are failing.
fn measure<F>(label: &str, mut request_fn: F) -> Vec<Duration>
where
    F: FnMut() -> (String, Duration),
{
    let mut durations = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        let (response, elapsed) = request_fn();
        let status = status_of(&response);
        assert!(
            status.starts_with('2'),
            "{label} sample {i} returned non-2xx status {status}: {response}"
        );
        durations.push(elapsed);
    }
    durations
}

fn median(durations: &mut [Duration]) -> Duration {
    durations.sort();
    durations[durations.len() / 2]
}

fn assert_median_under_sla(label: &str, mut durations: Vec<Duration>) {
    let max = *durations.iter().max().unwrap();
    let p50 = median(&mut durations);
    assert!(
        p50 < SLA,
        "{label}: median latency {p50:?} over {SAMPLES} samples exceeds the {SLA:?} SLA \
         (max observed: {max:?})"
    );
}

#[test]
fn health_check_latency_stays_under_sla() {
    let server = Server::spawn();
    let port = server.port;

    // Warm up: first request(s) after spawn can be slower (page faults,
    // lazy init) and would otherwise pollute the sample.
    let _ = http_call(port, "GET", "/health", "");

    let durations = measure("GET /health", || {
        let start = Instant::now();
        let response = http_call(port, "GET", "/health", "").expect("health request failed");
        (response, start.elapsed())
    });

    assert_median_under_sla("GET /health", durations);
}

#[test]
fn rz_parse_latency_stays_under_sla() {
    let server = Server::spawn();
    let port = server.port;
    let body = tool_call_body("rz_parse");

    let _ = http_call(port, "POST", "/mcp/call", &body);

    let durations = measure("rz_parse", || {
        let start = Instant::now();
        let response =
            http_call(port, "POST", "/mcp/call", &body).expect("rz_parse request failed");
        (response, start.elapsed())
    });

    assert_median_under_sla("rz_parse", durations);
}

#[test]
fn rz_typecheck_latency_stays_under_sla() {
    let server = Server::spawn();
    let port = server.port;
    let body = tool_call_body("rz_typecheck");

    let _ = http_call(port, "POST", "/mcp/call", &body);

    let durations = measure("rz_typecheck", || {
        let start = Instant::now();
        let response =
            http_call(port, "POST", "/mcp/call", &body).expect("rz_typecheck request failed");
        (response, start.elapsed())
    });

    assert_median_under_sla("rz_typecheck", durations);
}

#[test]
fn rz_run_latency_stays_under_sla() {
    let server = Server::spawn();
    let port = server.port;
    let body = tool_call_body("rz_run");

    let _ = http_call(port, "POST", "/mcp/call", &body);

    let durations = measure("rz_run", || {
        let start = Instant::now();
        let response = http_call(port, "POST", "/mcp/call", &body).expect("rz_run request failed");
        (response, start.elapsed())
    });

    assert_median_under_sla("rz_run", durations);
}
