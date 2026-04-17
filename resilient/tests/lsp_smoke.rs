//! RES-090: LSP integration smoke test.
//!
//! Spawns the `resilient` binary with `--lsp` and exchanges a
//! single `initialize` round-trip over LSP framing
//! (`Content-Length: N\r\n\r\n<json>`). Verifies the response is a
//! well-formed JSON-RPC reply that includes `capabilities`.
//!
//! Gated on `--features lsp` so the test is only compiled when the
//! LSP server is built. Hand-rolls the framing rather than pulling
//! in a JSON-RPC client crate — keeps the dep tree lean and the
//! test footprint small.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Frame a JSON payload as an LSP message:
/// `Content-Length: N\r\n\r\n<json>`.
fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

/// Read exactly one LSP message from `r`. Returns the JSON body
/// (excluding headers). Times out after `deadline` if the response
/// hasn't arrived.
fn read_one_message<R: Read>(r: &mut R, deadline: Instant) -> Result<String, String> {
    let mut header = Vec::<u8>::new();
    let mut content_length: Option<usize> = None;

    // Read until we've consumed `\r\n\r\n` AND parsed Content-Length.
    loop {
        if Instant::now() >= deadline {
            return Err("timed out waiting for LSP header".into());
        }
        let mut buf = [0u8; 1];
        let n = r.read(&mut buf).map_err(|e| format!("read error: {}", e))?;
        if n == 0 {
            return Err("unexpected EOF before LSP header complete".into());
        }
        header.push(buf[0]);
        // Look for `\r\n\r\n` terminator.
        if header.ends_with(b"\r\n\r\n") {
            let header_str = std::str::from_utf8(&header)
                .map_err(|e| format!("bad header utf8: {}", e))?;
            for line in header_str.split("\r\n") {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Content-Length:") {
                    content_length = Some(
                        rest.trim()
                            .parse::<usize>()
                            .map_err(|e| format!("bad Content-Length: {}", e))?,
                    );
                }
            }
            if content_length.is_some() {
                break;
            }
            return Err(format!("LSP header missing Content-Length: {:?}", header_str));
        }
    }

    let len = content_length.unwrap();
    let mut body = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out reading LSP body ({}/{} bytes)",
                filled, len
            ));
        }
        let n = r
            .read(&mut body[filled..])
            .map_err(|e| format!("body read error: {}", e))?;
        if n == 0 {
            return Err("unexpected EOF in LSP body".into());
        }
        filled += n;
    }
    String::from_utf8(body).map_err(|e| format!("bad body utf8: {}", e))
}

#[test]
fn lsp_initialize_round_trip() {
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // Step 1: initialize request. Minimal valid params.
    let init_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin
        .write_all(frame(init_body).as_bytes())
        .expect("write initialize");
    stdin.flush().ok();

    // Step 2: read response.
    let deadline = Instant::now() + Duration::from_secs(5);
    let response_body = read_one_message(&mut stdout, deadline).expect("read initialize response");

    // Verify the response shape via substring checks. Hand-rolling
    // beats pulling in serde_json as a direct dep.
    assert!(
        response_body.contains(r#""jsonrpc":"2.0""#),
        "missing jsonrpc:2.0 in: {response_body}"
    );
    assert!(
        response_body.contains(r#""id":1"#),
        "missing id:1 in: {response_body}"
    );
    assert!(
        response_body.contains(r#""capabilities""#),
        "missing capabilities in: {response_body}"
    );
    assert!(
        response_body.contains(r#""textDocumentSync""#),
        "missing textDocumentSync (proves Backend::initialize ran) in: {response_body}"
    );

    // Step 3: send `exit` notification so the server terminates
    // cleanly. tower-lsp's Server loop responds to it by returning
    // from .serve(), which lets the binary's tokio runtime drop and
    // the process exits.
    let exit_body = r#"{"jsonrpc":"2.0","method":"exit"}"#;
    let _ = stdin.write_all(frame(exit_body).as_bytes());
    drop(stdin);

    // Step 4: wait for the process to exit. Cap the wait so a stuck
    // server doesn't hang the test suite.
    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break, // exited
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("LSP server did not exit within 3s of exit notification");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("try_wait failed: {}", e),
        }
    }
}
