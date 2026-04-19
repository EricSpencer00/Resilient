//! RES-188a: LSP completion integration smoke test.
//!
//! Spawns `resilient --lsp`, opens a document with one top-level
//! fn, and drives `textDocument/completion` at three cursor
//! positions:
//!   - Mid-identifier (`prin|`) → expects `println` in the list.
//!   - Prefix matching user decl (`my_|`) → expects `my_helper`.
//!   - Empty prefix (Ctrl-Space) → expects a non-empty list.
//!
//! Mirrors `lsp_smoke.rs` / `lsp_hover_smoke.rs` / `lsp_goto_def_smoke.rs`
//! framing so the test dep tree stays empty. Gated on `--features lsp`.

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
    }
}

#[test]
fn lsp_completion_returns_builtins_and_top_level_decls() {
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let deadline = Instant::now() + Duration::from_secs(10);

    // ---- initialize ----
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let init_resp = read_until_id(&mut stdout, 1, deadline).unwrap();
    assert!(
        init_resp.contains("\"completionProvider\""),
        "expected `completionProvider` in initialize response, got:\n{}",
        init_resp,
    );

    // ---- initialized ----
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen ----
    // Document layout:
    //   line 0: `fn my_helper() { return 1; }`
    //   line 1: `let r = prin`       (<-- cursor at col 12 → prefix "prin")
    //   line 2: `let s = my_`        (<-- cursor at col 11 → prefix "my_")
    //   line 3: ``                    (<-- empty line, cursor for Ctrl-Space)
    let src = concat!(
        "fn my_helper() { return 1; }\n",
        "let r = prin\n",
        "let s = my_\n",
        "\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///comp.rs","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- completion request 1: mid "prin" ----
    let c1 = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///comp.rs"},"position":{"line":1,"character":12}}}"#;
    stdin.write_all(frame(c1).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let r1 = read_until_id(&mut stdout, 2, deadline).unwrap();
    assert!(
        r1.contains("\"println\""),
        "expected `println` in completion list:\n{}",
        r1,
    );
    assert!(
        r1.contains("\"print\""),
        "expected `print` in completion list:\n{}",
        r1,
    );

    // ---- completion request 2: `my_` (user decl prefix) ----
    let c2 = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///comp.rs"},"position":{"line":2,"character":11}}}"#;
    stdin.write_all(frame(c2).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let r2 = read_until_id(&mut stdout, 3, deadline).unwrap();
    assert!(
        r2.contains("\"my_helper\""),
        "expected `my_helper` in completion list:\n{}",
        r2,
    );

    // ---- completion request 3: empty prefix (Ctrl-Space) ----
    let c3 = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///comp.rs"},"position":{"line":3,"character":0}}}"#;
    stdin.write_all(frame(c3).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let r3 = read_until_id(&mut stdout, 4, deadline).unwrap();
    // Empty prefix returns a non-empty array — builtins at least.
    assert!(
        r3.contains("\"result\":["),
        "expected `result:[...]` array in completion list:\n{}",
        r3,
    );
    // And the array should actually have entries (not `[]`).
    assert!(
        !r3.contains("\"result\":[]"),
        "empty-prefix completion returned empty array:\n{}",
        r3,
    );

    // ---- shutdown ----
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
