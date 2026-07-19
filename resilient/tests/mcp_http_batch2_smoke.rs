//! Integration smoke tests for RES-3937 (bounded concurrency), RES-3941
//! (structured request logging), and RES-3942 (graceful SIGTERM shutdown)
//! on the MCP HTTP wrapper.
//!
//! These spawn the real `rz` binary and talk to it over a real TCP
//! socket / real OS process signal, so they exercise the same code
//! path a deployed instance would.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// Ask the OS for an unused port by binding to `:0`, then dropping the
/// listener before the server binds it for real. Small TOCTOU race in
/// theory; fine for a test running in an isolated CI sandbox.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn spawn(extra_env: &[(&str, &str)]) -> Self {
        let port = free_port();
        let mut cmd = Command::new(bin());
        cmd.arg("mcp")
            .arg("--http-port")
            .arg(format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("failed to spawn rz mcp --http-port");
        let server = Server { child, port };
        server.wait_ready();
        server
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok() {
                return;
            }
            if Instant::now() > deadline {
                panic!("server on port {} never became ready", self.port);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn take_stderr(&mut self) -> std::process::ChildStderr {
        self.child.stderr.take().expect("stderr was piped")
    }

    /// Send SIGTERM to the child process (Unix only — this whole hardening
    /// batch targets the host HTTP wrapper, which only ships for Unix
    /// hosts today).
    #[cfg(unix)]
    fn terminate(&self) {
        unsafe {
            libc_kill(self.child.id() as i32, 15 /* SIGTERM */);
        }
    }
}

// RES-3942: minimal raw FFI binding to `kill(2)`, mirroring the
// production code's no-new-deps approach to signal delivery — this is
// test-only code, sending the signal a real deployment's process
// supervisor (systemd, Docker, Kubernetes) would send on shutdown.
#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build and send a raw HTTP/1.1 request over `stream`, returning the
/// full response text. No dependency on an HTTP client crate.
fn http_call(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to MCP HTTP server");
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn status_of(response: &str) -> &str {
    response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?")
}

/// Resilient source that burns enough wall-clock time (a busy loop, no
/// stdlib `sleep` dependency needed) to make overlap-vs-serial timing
/// unambiguous, without needing a fixed-latency primitive.
const SLOW_SOURCE: &str = r#"
fn main() -> int {
    let i: int = 0;
    while i < 60000000 {
        i = i + 1;
    }
    return i;
}
main();
"#;

fn slow_call_body() -> String {
    serde_json::json!({
        "tool": "rz_run",
        "input": { "source": SLOW_SOURCE }
    })
    .to_string()
}

#[test]
fn concurrent_requests_overlap() {
    let server = Server::spawn(&[
        ("RESILIENT_MCP_MAX_CONNECTIONS", "4"),
        ("RESILIENT_MCP_TIMEOUT_SECS", "30"),
    ]);
    let port = server.port;
    let body = slow_call_body();

    // Time one request alone to establish a per-request baseline.
    let solo_start = Instant::now();
    let resp = http_call(port, "POST", "/mcp/call", &body);
    let solo_elapsed = solo_start.elapsed();
    assert_eq!(status_of(&resp), "200", "solo request failed: {resp}");

    // Fire two more of the same slow request concurrently. If the server
    // still served connections one-at-a-time (RES-3937 regression), the
    // wall-clock time for both would be roughly 2x the solo baseline. A
    // bounded worker pool overlaps them, so total time should stay much
    // closer to the single-request baseline.
    let body_a = body.clone();
    let body_b = body.clone();
    let concurrent_start = Instant::now();
    let t1 = std::thread::spawn(move || http_call(port, "POST", "/mcp/call", &body_a));
    let t2 = std::thread::spawn(move || http_call(port, "POST", "/mcp/call", &body_b));
    let r1 = t1.join().unwrap();
    let r2 = t2.join().unwrap();
    let concurrent_elapsed = concurrent_start.elapsed();

    assert_eq!(status_of(&r1), "200", "concurrent request 1 failed: {r1}");
    assert_eq!(status_of(&r2), "200", "concurrent request 2 failed: {r2}");

    // Overlap evidence: two concurrent slow requests should finish in
    // well under 2x the solo time (generous 1.6x threshold to absorb CI
    // scheduling noise) — true serialization would land near 2x.
    let threshold = solo_elapsed.as_secs_f64() * 1.6;
    assert!(
        concurrent_elapsed.as_secs_f64() < threshold,
        "concurrent requests did not overlap: solo={solo_elapsed:?} concurrent={concurrent_elapsed:?} threshold<{threshold:?}s",
    );
}

#[test]
fn log_line_shape_has_expected_fields() {
    let mut server = Server::spawn(&[]);
    let stderr = server.take_stderr();
    let port = server.port;

    let resp = http_call(port, "GET", "/health", "");
    assert_eq!(status_of(&resp), "200", "health check failed: {resp}");

    // Give the log line a moment to land, then read whatever stderr has
    // buffered so far.
    std::thread::sleep(Duration::from_millis(200));
    drop(server); // triggers shutdown-log lines too; we only need the request line below.

    let mut buf = String::new();
    let mut stderr = stderr;
    let _ = stderr.read_to_string(&mut buf);

    let log_line = buf
        .lines()
        .find(|l| l.contains("path=/health"))
        .unwrap_or_else(|| panic!("no access-log line for /health found in stderr:\n{buf}"));

    for field in [
        "ts_ms=",
        "peer=",
        "method=GET",
        "path=/health",
        "status=200",
        "duration_ms=",
        "bytes=",
    ] {
        assert!(
            log_line.contains(field),
            "log line missing `{field}`: {log_line}"
        );
    }
}

#[cfg(unix)]
#[test]
fn shutdown_drains_in_flight_request() {
    let mut server = Server::spawn(&[
        ("RESILIENT_MCP_TIMEOUT_SECS", "30"),
        ("RESILIENT_MCP_SHUTDOWN_DRAIN_SECS", "20"),
    ]);
    let port = server.port;
    let body = slow_call_body();

    // Kick off a slow request in the background, give it a moment to be
    // accepted by a worker, then send SIGTERM. The in-flight request
    // should still complete successfully (drained, not dropped), and the
    // process should exit cleanly afterward.
    let handle = std::thread::spawn(move || http_call(port, "POST", "/mcp/call", &body));
    std::thread::sleep(Duration::from_millis(300));
    server.terminate();

    let response = handle.join().expect("request thread panicked");
    assert_eq!(
        status_of(&response),
        "200",
        "in-flight request was dropped instead of drained: {response}"
    );

    // The process should exit on its own (not need to be killed) within
    // the drain deadline plus slack. `Server::drop`'s `kill()` on an
    // already-exited pid is a harmless no-op, so no special teardown is
    // needed here.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match server.child.try_wait().expect("try_wait") {
            Some(status) => {
                assert!(status.success(), "server exited non-zero: {status:?}");
                break;
            }
            None => {
                if Instant::now() > deadline {
                    panic!("server did not exit within the drain deadline after SIGTERM");
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}
