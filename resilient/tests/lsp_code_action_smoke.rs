//! RES-357: LSP code action — add contract stubs.
//!
//! Spawns `resilient --lsp`, opens a document containing a function
//! with no `requires`/`ensures` contract, issues a
//! `textDocument/codeAction` request for the L0010 diagnostic range,
//! and asserts the response contains a "Add contract stubs" action
//! whose `WorkspaceEdit` inserts `requires true;` and `ensures true;`.
//!
//! Mirrors the framing of `lsp_smoke.rs` / `lsp_hover_smoke.rs`
//! (hand-rolled LSP framing, no extra deps). Gated on
//! `--features lsp`.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

fn read_one_message<R: Read>(r: &mut R, deadline: Instant) -> Result<String, String> {
    let mut header = Vec::<u8>::new();
    let mut content_length: Option<usize> = None;
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
        if header.ends_with(b"\r\n\r\n") {
            let header_str =
                std::str::from_utf8(&header).map_err(|e| format!("bad header utf8: {}", e))?;
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
            return Err(format!(
                "LSP header missing Content-Length: {:?}",
                header_str
            ));
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

fn read_until_id<R: Read>(
    r: &mut R,
    expected_id: u64,
    deadline: Instant,
) -> Result<String, String> {
    loop {
        let body = read_one_message(r, deadline)?;
        if body.contains(&format!("\"id\":{}", expected_id)) {
            return Ok(body);
        }
        // skip notifications and other replies
    }
}

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
    }
}

/// RES-357 AC: `textDocument/codeAction` for an L0010 diagnostic returns
/// a "Add contract stubs" action whose edit inserts `requires true;` and
/// `ensures true;` after the opening `{` of the function.
#[test]
fn lsp_code_action_add_contract_stubs() {
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let deadline = Instant::now() + Duration::from_secs(15);

    // ---- initialize ----
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let init_resp = read_until_id(&mut stdout, 1, deadline).unwrap();
    assert!(
        init_resp.contains(r#""codeActionProvider""#),
        "expected codeActionProvider in capabilities, got:\n{}",
        init_resp
    );

    // ---- initialized ----
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen a source file with a no-contract function ----
    // The function `foo` has neither `requires` nor `ensures`, so
    // L0010 should fire.  The lint is surface-level; the server
    // publishes it via the lint pipeline wired into `publish_analysis`.
    //
    // Source text (escaping for JSON):
    //   fn foo(int x) { return x; }
    let uri = "file:///tmp/lsp_code_action_test.rs";
    let src_escaped = r#"fn foo(int x) { return x; }"#;
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"{src_escaped}"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Drain publishDiagnostics (may or may not contain the lint warning;
    // the test doesn't rely on the diagnostic being published — it sends
    // its own synthetic diagnostic in the codeAction request below).
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        deadline,
    )
    .expect("read publishDiagnostics");

    // ---- textDocument/codeAction ----
    // Send a code action request with a synthetic L0010 diagnostic
    // covering line 0 of the document.  The server should reply with
    // a "Add contract stubs" action.
    let diag_message = r#"function `foo` has no `requires`\/`ensures` contract; add contract stubs or suppress with `\/\/ resilient: allow L0010`"#;
    let code_action_req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/codeAction","params":{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":5}}}},"context":{{"diagnostics":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":5}}}},"severity":2,"source":"resilient-lint","message":"function `foo` has no `requires`/`ensures` contract; add contract stubs or suppress with `// resilient: allow L0010`"}}],"only":null}}}}}}"#
    );
    // Note: the message above uses plain `/` and backtick chars which are
    // valid JSON string characters.  We re-build it cleanly:
    let diag_json = r#"{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":5}},"severity":2,"source":"resilient-lint","message":"function `foo` has no `requires`/`ensures` contract; add contract stubs or suppress with `// resilient: allow L0010`"}"#;
    let ca_req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/codeAction","params":{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":5}}}},"context":{{"diagnostics":[{diag_json}]}}}}}}"#
    );
    let _ = diag_message; // suppress unused warning
    let _ = code_action_req;
    stdin.write_all(frame(&ca_req).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let response = read_until_id(&mut stdout, 2, deadline).expect("read codeAction response");

    // The response must contain the action title.
    assert!(
        response.contains("Add contract stubs"),
        "expected 'Add contract stubs' in codeAction response:\n{response}"
    );
    // The edit must include both contract stub lines.
    assert!(
        response.contains("requires true;"),
        "expected `requires true;` in codeAction edit:\n{response}"
    );
    assert!(
        response.contains("ensures true;"),
        "expected `ensures true;` in codeAction edit:\n{response}"
    );
    // The action kind should be "quickfix".
    assert!(
        response.contains("quickfix"),
        "expected `quickfix` kind in codeAction response:\n{response}"
    );

    // ---- clean shutdown ----
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

/// RES-357 AC: when no L0010 diagnostic is present in the request
/// context, the server returns null (no actions).
#[test]
fn lsp_code_action_returns_null_for_non_l0010_diagnostic() {
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let deadline = Instant::now() + Duration::from_secs(15);

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let _ = read_until_id(&mut stdout, 1, deadline).unwrap();

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let uri = "file:///tmp/lsp_ca_null_test.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"let x = 1;"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        deadline,
    )
    .expect("read publishDiagnostics");

    // Send a codeAction request with an unrelated diagnostic (no L0010).
    let ca_req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/codeAction","params":{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":3}}}},"context":{{"diagnostics":[{{"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":3}}}},"message":"some unrelated error"}}]}}}}}}"#
    );
    stdin.write_all(frame(&ca_req).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let response = read_until_id(&mut stdout, 2, deadline).expect("read codeAction response");

    // No matching diagnostic → result must be null.
    assert!(
        response.contains(r#""result":null"#),
        "expected null result when no L0010 diagnostic present:\n{response}"
    );

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
