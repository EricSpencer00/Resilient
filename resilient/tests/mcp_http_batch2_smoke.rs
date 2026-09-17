//! Integration smoke tests for RES-3937 (bounded concurrency), RES-3941
//! (structured request logging), and RES-3942 (graceful SIGTERM shutdown)
//! on the MCP HTTP wrapper.
//!
//! These spawn the real `rz` binary and talk to it over a real TCP
//! socket / real OS process signal, so they exercise the same code
//! path a deployed instance would.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    fn spawn(extra_env: &[(&str, &str)]) -> Self {
        let handle = match spawn_with_retry(|port| {
            let mut cmd = Command::new(bin());
            cmd.arg("mcp")
                .arg("--http-port")
                .arg(format!("127.0.0.1:{port}"))
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            for (k, v) in extra_env {
                cmd.env(k, v);
            }
            cmd.spawn()
        }) {
            Ok(handle) => handle,
            Err(err) => panic!("{err}"),
        };
        Server { handle }
    }

    fn port(&self) -> u16 {
        self.handle.port
    }

    fn take_stderr(&mut self) -> std::process::ChildStderr {
        self.handle.child.stderr.take().expect("stderr was piped")
    }

    /// Send SIGTERM to the child process (Unix only — this whole hardening
    /// batch targets the host HTTP wrapper, which only ships for Unix
    /// hosts today).
    #[cfg(unix)]
    fn terminate(&self) {
        unsafe {
            libc_kill(self.handle.child.id() as i32, 15 /* SIGTERM */);
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
        let _ = self.handle.child.kill();
        let _ = self.handle.child.wait();
    }
}

/// Send an HTTP/1.1 request through the shared retry helper.
fn http_call_result(port: u16, method: &str, path: &str, body: &str) -> Result<String, String> {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    send_request_retrying(
        port,
        &request,
        Duration::from_secs(15),
        DEFAULT_READY_DEADLINE,
    )
}

fn http_call(port: u16, method: &str, path: &str, body: &str) -> String {
    http_call_result(port, method, path, body)
        .unwrap_or_else(|err| panic!("HTTP request failed: {err}"))
}

fn status_of(response: &str) -> &str {
    response
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("?")
}

fn marker_path(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "resilient-mcp-batch2-{}-{label}-{nanos}",
        std::process::id()
    ))
}

fn rz_string(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Resilient source that waits on a host-file barrier. The request cannot
/// complete until the test releases it, making overlap independent of host
/// scheduling speed and avoiding timing thresholds.
fn slow_source(start_path: &Path, release_path: &Path) -> String {
    let start_path = rz_string(start_path);
    let release_path = rz_string(release_path);
    format!(
        r#"
fn main() -> int {{
    file_write("{start_path}", "started");
    while file_exists("{release_path}") == false {{
        let spin: int = 0;
        while spin < 1000 {{
            spin = spin + 1;
        }}
    }}
    return 0;
}}
main();
"#
    )
}

fn slow_call_body(start_path: &Path, release_path: &Path) -> String {
    serde_json::json!({
        "tool": "rz_run",
        "input": { "source": slow_source(start_path, release_path) }
    })
    .to_string()
}

fn wait_for_marker(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        path.exists(),
        "slow request did not reach its barrier: {}",
        path.display()
    );
}

#[test]
fn concurrent_requests_overlap() {
    let server = Server::spawn(&[
        ("RESILIENT_MCP_MAX_CONNECTIONS", "4"),
        ("RESILIENT_MCP_TIMEOUT_SECS", "30"),
    ]);
    let port = server.port();
    let start_path = marker_path("overlap-start");
    let release_path = marker_path("overlap-release");
    let _ = fs::remove_file(&start_path);
    let _ = fs::remove_file(&release_path);
    let body = slow_call_body(&start_path, &release_path);

    let slow_done = Arc::new(AtomicBool::new(false));
    let slow_done_thread = Arc::clone(&slow_done);
    let slow_body = body.clone();
    let slow_handle = std::thread::spawn(move || {
        let result = http_call_result(port, "POST", "/mcp/call", &slow_body);
        slow_done_thread.store(true, Ordering::Release);
        result
    });

    wait_for_marker(&start_path);

    let health_done = Arc::new(AtomicBool::new(false));
    let health_done_thread = Arc::clone(&health_done);
    let health_handle = std::thread::spawn(move || {
        let result = http_call_result(port, "GET", "/health", "");
        health_done_thread.store(true, Ordering::Release);
        result
    });

    // The slow request is held open by the barrier. A concurrent worker must
    // answer the independent health request before the barrier is released;
    // a serialized server cannot do so without waiting for the slow request.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !health_done.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let overlapped = health_done.load(Ordering::Acquire) && !slow_done.load(Ordering::Acquire);

    fs::write(&release_path, "release").expect("release slow request");
    let slow_response = slow_handle
        .join()
        .expect("slow request thread panicked")
        .expect("slow request failed");
    let health_response = health_handle
        .join()
        .expect("health request thread panicked")
        .expect("health request failed");

    assert_eq!(
        status_of(&slow_response),
        "200",
        "slow request failed: {slow_response}"
    );
    assert_eq!(
        status_of(&health_response),
        "200",
        "health request failed: {health_response}"
    );
    assert!(
        overlapped,
        "health request did not complete while slow request was in flight"
    );
    let _ = fs::remove_file(&start_path);
    let _ = fs::remove_file(&release_path);
}

#[test]
fn log_line_shape_has_expected_fields() {
    let mut server = Server::spawn(&[]);
    let stderr = server.take_stderr();
    let port = server.port();

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
    let port = server.port();
    let start_path = marker_path("shutdown-start");
    let release_path = marker_path("shutdown-release");
    let _ = fs::remove_file(&start_path);
    let _ = fs::remove_file(&release_path);
    let body = slow_call_body(&start_path, &release_path);

    // Hold a request at a known in-flight point, then send SIGTERM. The
    // request should still complete successfully (drained, not dropped),
    // and the process should exit cleanly afterward.
    let handle = std::thread::spawn(move || http_call_result(port, "POST", "/mcp/call", &body));
    wait_for_marker(&start_path);
    server.terminate();
    fs::write(&release_path, "release").expect("release in-flight request");

    let response = handle
        .join()
        .expect("request thread panicked")
        .expect("in-flight request failed");
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
        match server.handle.child.try_wait().expect("try_wait") {
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
    let _ = fs::remove_file(&start_path);
    let _ = fs::remove_file(&release_path);
}
