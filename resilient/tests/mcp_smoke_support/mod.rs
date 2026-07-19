//! RES-4204: shared retry helpers for MCP HTTP smoke tests.
//!
//! Every smoke test that spawns the real `rz mcp --http-port` binary and
//! talks to it over TCP hits the same two startup-race hazards:
//!
//! 1. The OS process is spawned asynchronously, so the very first connect
//!    attempt can land before the listener socket is accepting connections
//!    at all (`ConnectionRefused`).
//! 2. Even once the listener accepts connections, the request-handling
//!    loop behind it may not be fully up yet under CI contention (e.g. the
//!    `--features z3` leg, which is measurably slower to start): a connect
//!    can succeed and then the peer resets the connection before a
//!    response is written (`ConnectionReset`).
//!
//! A fixed sleep or a single connect/request attempt is flaky under CI
//! load either way (too short: hits one of the above; too long: wasted
//! wall-clock). Instead, poll with a bounded deadline and a short backoff,
//! retrying the *whole* request round-trip on transient I/O errors rather
//! than only the initial connect.
//!
//! Lives at `tests/mcp_smoke_support/mod.rs` (a subdirectory, not a
//! top-level `tests/*.rs` file) so cargo's integration-test discovery does
//! not treat it as its own test binary; each consumer pulls it in with
//! `#[path = "mcp_smoke_support/mod.rs"] mod mcp_smoke_support;`.
//!
//! All of these return `Result` rather than panicking directly — callers
//! panic with test-local context (which port, which server) at the call
//! site, keeping this module a plain library helper.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// Default deadline for waiting on the MCP HTTP server to become ready /
/// for retrying a request. Generous enough to absorb slow/contended CI
/// runners (the `--features z3` leg in particular).
pub const DEFAULT_READY_DEADLINE: Duration = Duration::from_secs(10);

/// Backoff between retry attempts while polling.
const RETRY_BACKOFF: Duration = Duration::from_millis(25);

/// A raw HTTP/1.1 request/response error worth retrying: the connection
/// never got established, or it was torn down mid-exchange. Anything else
/// (a malformed response, a timeout well past startup) is a real test
/// failure and should not be silently retried away.
fn is_transient(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::TimedOut
    )
}

/// Poll `127.0.0.1:{port}` with plain TCP connects until one succeeds or
/// `deadline` elapses.
///
/// This only proves the listener is accepting connections, not that the
/// request-handling loop behind it is live — prefer [`wait_for_health`]
/// when the server under test exposes `/health`, and always send the
/// actual request through [`send_request_retrying`] rather than a single
/// bare attempt.
pub fn wait_until_ready(port: u16, deadline: Duration) -> Result<(), String> {
    let start = Instant::now();
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        if start.elapsed() > deadline {
            return Err(format!(
                "server on 127.0.0.1:{port} never became ready within {deadline:?}"
            ));
        }
        std::thread::sleep(RETRY_BACKOFF);
    }
}

/// Poll `GET /health` until it returns a well-formed HTTP response (not
/// necessarily status 200 — callers that want a specific status should
/// check the returned body themselves) or `deadline` elapses.
///
/// This is a stronger readiness signal than [`wait_until_ready`]: it
/// proves a full request/response round-trip succeeds, not just that the
/// listener backlog will accept a SYN.
pub fn wait_for_health(port: u16, deadline: Duration) -> Result<String, String> {
    let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    send_request_retrying(port, request, Duration::from_secs(5), deadline)
}

/// Send a raw HTTP/1.1 `request` to `127.0.0.1:{port}` and return the full
/// response text, retrying the whole connect+write+read round-trip on
/// transient I/O errors (connection refused/reset/aborted/timed-out) until
/// `deadline` elapses.
///
/// This is the retry unit that actually matters for the flake this module
/// exists to fix: a prior successful readiness check does not guarantee
/// the *next* connection survives to a response (see module docs), so
/// every request a smoke test makes should go through this, not just the
/// initial "is the server up" check.
pub fn send_request_retrying(
    port: u16,
    request: &str,
    read_timeout: Duration,
    deadline: Duration,
) -> Result<String, String> {
    let start = Instant::now();
    loop {
        match try_once(port, request, read_timeout) {
            Ok(response) => return Ok(response),
            Err(err) if is_transient(&err) && start.elapsed() <= deadline => {
                std::thread::sleep(RETRY_BACKOFF);
            }
            Err(err) => {
                return Err(format!(
                    "request to 127.0.0.1:{port} failed after {:?}: {err}",
                    start.elapsed()
                ));
            }
        }
    }
}

fn try_once(port: u16, request: &str, read_timeout: Duration) -> std::io::Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(read_timeout))?;
    stream.write_all(request.as_bytes())?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}
