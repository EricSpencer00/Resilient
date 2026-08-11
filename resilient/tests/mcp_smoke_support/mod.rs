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
use std::net::{TcpListener, TcpStream};
use std::process::Child;
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

/// A spawned `rz mcp --http-port` child together with the port it bound.
pub struct ServerHandle {
    pub child: Child,
    pub port: u16,
}

/// Ask the OS for an unused port by binding `:0` and dropping the listener.
///
/// Inherently racy: the port is only reserved until the listener drops, and
/// the child re-binds it a moment later. [`spawn_with_retry`] is what makes
/// that race survivable — do not call this directly to spawn a server.
pub fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().unwrap().port()
}

/// How many fresh ports to try before giving up on a bind collision.
const SPAWN_ATTEMPTS: usize = 5;

/// Spawn the MCP HTTP server on a free port and return once it serves a
/// real `/health` round-trip, re-spawning on a fresh port if the child
/// loses a bind race.
///
/// RES-4224: [`free_port`] has an unavoidable TOCTOU window — it drops the
/// probe listener before the child binds. `cargo test` runs the MCP smoke
/// binaries in parallel, each spawning several servers, so two of them can
/// be handed the same port. The loser exits immediately:
///
/// ```text
/// MCP HTTP server failed: Address already in use (os error 48)
/// ```
///
/// leaving the test polling a port nothing is listening on until its
/// deadline expires. Retrying the *request* cannot fix that, because there
/// is no longer a server to reach — the process must be re-spawned
/// somewhere else. So this watches for child exit and treats it as "lost
/// the race, try another port", while a child that stays alive but never
/// serves `/health` is reported as the genuine failure it is.
///
/// `launch` receives the chosen port and returns the spawned child, so
/// callers keep control of env vars and stdio wiring.
pub fn spawn_with_retry<F>(mut launch: F) -> Result<ServerHandle, String>
where
    F: FnMut(u16) -> std::io::Result<Child>,
{
    let mut last_err = String::new();
    for _ in 0..SPAWN_ATTEMPTS {
        let port = free_port();
        let mut child = match launch(port) {
            Ok(child) => child,
            Err(err) => return Err(format!("failed to spawn rz mcp --http-port: {err}")),
        };
        match wait_for_health_while_alive(&mut child, port, DEFAULT_READY_DEADLINE) {
            Ok(()) => return Ok(ServerHandle { child, port }),
            Err(WaitFailure::ChildExited(status)) => {
                let _ = child.wait();
                last_err = format!("child on port {port} exited early ({status})");
            }
            Err(WaitFailure::Timeout(err)) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err);
            }
        }
    }
    Err(format!(
        "server never came up after {SPAWN_ATTEMPTS} attempts on fresh ports; last: {last_err}"
    ))
}

enum WaitFailure {
    /// The child is gone — almost always a lost bind race. Retryable.
    ChildExited(String),
    /// The child is alive but not serving. A real failure; do not retry.
    Timeout(String),
}

fn wait_for_health_while_alive(
    child: &mut Child,
    port: u16,
    deadline: Duration,
) -> Result<(), WaitFailure> {
    let start = Instant::now();
    let request = "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n";
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(WaitFailure::ChildExited(status.to_string()));
        }
        if try_once(port, request, Duration::from_secs(5)).is_ok() {
            return Ok(());
        }
        if start.elapsed() > deadline {
            return Err(WaitFailure::Timeout(format!(
                "server on 127.0.0.1:{port} was still running but never served /health within {deadline:?}"
            )));
        }
        std::thread::sleep(RETRY_BACKOFF);
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
