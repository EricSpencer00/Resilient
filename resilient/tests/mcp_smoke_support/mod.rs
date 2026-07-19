//! RES-4204: shared retry-connect support for MCP HTTP smoke tests.
//!
//! Every smoke test that spawns the real `rz mcp --http-port` binary and
//! talks to it over TCP needs the same startup-race protection: the OS
//! process is spawned asynchronously, so the very first connect attempt
//! can land before the listener socket is accepting connections. A fixed
//! sleep is flaky under CI load (too short: connection-refused; too long:
//! wasted wall-clock). Instead, poll with a bounded deadline and a short
//! backoff between attempts.
//!
//! Lives at `tests/mcp_smoke_support/mod.rs` (a subdirectory, not a
//! top-level `tests/*.rs` file) so cargo's integration-test discovery does
//! not treat it as its own test binary; each consumer pulls it in with
//! `#[path = "mcp_smoke_support/mod.rs"] mod mcp_smoke_support;`.

#![allow(dead_code)]

use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Default deadline for waiting on the MCP HTTP server to start accepting
/// connections. Generous enough to absorb slow/contended CI runners.
pub const DEFAULT_READY_DEADLINE: Duration = Duration::from_secs(10);

/// Backoff between retry attempts while polling for readiness.
const RETRY_BACKOFF: Duration = Duration::from_millis(25);

/// Poll `127.0.0.1:{port}` with plain TCP connects until one succeeds or
/// `deadline` elapses, at which point it panics with a diagnostic message.
///
/// This only proves the listener is accepting connections — callers that
/// immediately issue a real request should still treat a transient
/// connection failure as retryable (see [`connect_retrying`]) rather than
/// assuming readiness is permanent, since some CI sandboxes briefly reset
/// connections around process/network setup even after the listener is up.
pub fn wait_until_ready(port: u16, deadline: Duration) {
    let start = Instant::now();
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if start.elapsed() > deadline {
            panic!(
                "server on 127.0.0.1:{port} never became ready within {:?}",
                deadline
            );
        }
        std::thread::sleep(RETRY_BACKOFF);
    }
}

/// Connect to `127.0.0.1:{port}`, retrying on connection-refused (and other
/// transient connect errors) until `deadline` elapses.
///
/// Use this for the *actual* request connection in a test, not just the
/// initial readiness check — a successful `wait_until_ready` call does not
/// guarantee every subsequent connect succeeds immediately (accept-queue
/// pressure, scheduler noise), and retrying here is what actually kills the
/// class of intermittent connection-refused flake this helper exists for.
pub fn connect_retrying(port: u16, deadline: Duration) -> TcpStream {
    let start = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => return stream,
            Err(err) => {
                if start.elapsed() > deadline {
                    panic!(
                        "could not connect to 127.0.0.1:{port} within {:?}: {err}",
                        deadline
                    );
                }
                std::thread::sleep(RETRY_BACKOFF);
            }
        }
    }
}

/// Poll `GET /health` until it returns any well-formed HTTP response (not
/// necessarily a 200 — callers that want a specific status should check the
/// returned body themselves) or `deadline` elapses.
///
/// This is a stronger readiness signal than a bare TCP connect: it proves
/// the server's request-handling loop is live, not just that the listener
/// backlog will accept a SYN.
pub fn wait_for_health(port: u16, deadline: Duration) -> String {
    let start = Instant::now();
    loop {
        if let Some(response) = try_health_once(port) {
            return response;
        }
        if start.elapsed() > deadline {
            panic!("GET /health on 127.0.0.1:{port} never succeeded within {deadline:?}");
        }
        std::thread::sleep(RETRY_BACKOFF);
    }
}

fn try_health_once(port: u16) -> Option<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    stream.write_all_or_none(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    if response.starts_with("HTTP/1.1 ") || response.starts_with("HTTP/1.0 ") {
        Some(response)
    } else {
        None
    }
}

trait WriteOrNone {
    fn write_all_or_none(&mut self, buf: &[u8]) -> Option<()>;
}

impl WriteOrNone for TcpStream {
    fn write_all_or_none(&mut self, buf: &[u8]) -> Option<()> {
        use std::io::Write;
        self.write_all(buf).ok()
    }
}
