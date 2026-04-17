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

/// Read framed messages until one matches `predicate`, or the
/// `deadline` passes. Used by the didOpen test to skip past the
/// `initialize` response and any informational `window/logMessage`
/// notifications before reaching `publishDiagnostics`.
fn read_until<R: Read>(
    r: &mut R,
    predicate: impl Fn(&str) -> bool,
    deadline: Instant,
) -> Result<String, String> {
    loop {
        if Instant::now() >= deadline {
            return Err("timed out waiting for matching LSP message".into());
        }
        let body = read_one_message(r, deadline)?;
        if predicate(&body) {
            return Ok(body);
        }
        // else: skip and keep reading
    }
}

#[test]
fn lsp_did_open_publishes_diagnostics() {
    // RES-093: full didOpen → publishDiagnostics flow.
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // Step 1: initialize handshake.
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).expect("write initialize");
    stdin.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(5);
    // Drain the initialize response (id=1).
    let _init_resp = read_one_message(&mut stdout, deadline).expect("read initialize response");

    // Step 2: initialized notification.
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).expect("write initialized");

    // Step 3: didOpen with a 3-line program where line 3 is a known
    // type error. The typechecker rejects `let bad: int = "hi";`
    // (RES-053), and RES-080 prefixes the message with `<uri>:3:5:`.
    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/lsp_test.rs","languageId":"resilient","version":1,"text":"let a = 1;\nlet b = 2;\nlet bad: int = \"hi\";"}}}"#;
    stdin.write_all(frame(did_open).as_bytes()).expect("write didOpen");
    stdin.flush().ok();

    // Step 4: read until publishDiagnostics arrives. Skip log
    // messages or anything else the server might emit first.
    let diag_body = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    // Substring assertions on the notification body.
    assert!(
        diag_body.contains(r#""diagnostics""#),
        "missing diagnostics array in: {diag_body}"
    );
    // Source line 3 → 0-indexed LSP line 2. Allow `"line":2` or
    // `"line": 2` (whitespace tolerance).
    let has_line_2 = diag_body.contains(r#""line":2"#)
        || diag_body.contains(r#""line": 2"#);
    assert!(
        has_line_2,
        "expected `\"line\":2` (source line 3, 0-indexed) in: {diag_body}"
    );
    // The typechecker error wording mentions `let bad: int` per
    // RES-053. RES-080 prefixes it with the URI; the LSP
    // extractor strips that prefix back off, so the published
    // message should contain `let bad: int` somewhere.
    assert!(
        diag_body.contains("let bad: int") || diag_body.contains("string"),
        "expected typechecker-error wording in: {diag_body}"
    );

    // Step 5: clean shutdown.
    let exit = r#"{"jsonrpc":"2.0","method":"exit"}"#;
    let _ = stdin.write_all(frame(exit).as_bytes());
    drop(stdin);

    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("LSP server did not exit within 3s");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("try_wait failed: {}", e),
        }
    }
}

#[test]
fn lsp_did_change_republishes_diagnostics() {
    // RES-094: simulate the editor flow — clean program → buggy
    // edit → fixed edit. Each transition triggers a fresh
    // publishDiagnostics, and the empty-array publication on a
    // clean buffer is the LSP's "all clear" signal.
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // initialize
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().ok();
    let deadline = Instant::now() + Duration::from_secs(5);
    let _ = read_one_message(&mut stdout, deadline).expect("read initialize response");

    // initialized
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();

    // didOpen with a clean program
    let uri = "file:///tmp/lsp_change.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"let x = 1;"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    let is_diag = |body: &str| body.contains(r#""method":"textDocument/publishDiagnostics""#);

    // Step 1: clean program → empty diagnostics
    let body =
        read_until(&mut stdout, is_diag, Instant::now() + Duration::from_secs(5))
            .expect("read clean publishDiagnostics");
    assert!(
        body.contains(r#""diagnostics":[]"#),
        "expected EMPTY diagnostics for clean program; got:\n{body}"
    );

    // Step 2: didChange to a buggy version (FULL sync = full text replace)
    let did_change_buggy = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":"let bad: int = \"hi\";"}}]}}}}"#
    );
    stdin.write_all(frame(&did_change_buggy).as_bytes()).unwrap();
    stdin.flush().ok();

    let body =
        read_until(&mut stdout, is_diag, Instant::now() + Duration::from_secs(5))
            .expect("read buggy publishDiagnostics");
    assert!(
        !body.contains(r#""diagnostics":[]"#),
        "expected NON-empty diagnostics for buggy program; got:\n{body}"
    );
    assert!(
        body.contains("let bad: int") || body.contains("string"),
        "expected typechecker wording in: {body}"
    );

    // Step 3: didChange reverting to the clean version → empty again
    let did_change_clean = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":3}},"contentChanges":[{{"text":"let x = 1;"}}]}}}}"#
    );
    stdin.write_all(frame(&did_change_clean).as_bytes()).unwrap();
    stdin.flush().ok();

    let body =
        read_until(&mut stdout, is_diag, Instant::now() + Duration::from_secs(5))
            .expect("read fixed publishDiagnostics");
    assert!(
        body.contains(r#""diagnostics":[]"#),
        "expected EMPTY diagnostics after revert; got:\n{body}"
    );

    // exit + clean shutdown
    let exit = r#"{"jsonrpc":"2.0","method":"exit"}"#;
    let _ = stdin.write_all(frame(exit).as_bytes());
    drop(stdin);

    let exit_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() > exit_deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("LSP server did not exit within 3s");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("try_wait failed: {}", e),
        }
    }
}
