//! RES-183: LSP find-references integration smoke test.
//!
//! Spawns `resilient --lsp`, opens a document with a fn declared once
//! and called three times (plus a struct literal that uses the same
//! name but must NOT appear as a reference), then drives
//! `textDocument/references` at a cursor on the fn name. Asserts the
//! expected number of locations, that the struct literal is absent,
//! and that `includeDeclaration` adds the defining site.
//!
//! Mirrors `lsp_goto_def_smoke.rs` / `lsp_hover_smoke.rs` framing so
//! the test dep tree stays empty. Gated on `--features lsp`.

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
    }
}

/// Count the number of non-overlapping occurrences of `needle` in `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

#[test]
fn lsp_references_three_callers_struct_literal_excluded() {
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
        init_resp.contains("\"referencesProvider\""),
        "expected `referencesProvider` in initialize response, got:\n{}",
        init_resp,
    );

    // ---- initialized ----
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen a document ----
    // Layout (0-indexed LSP lines):
    //   line 0: `fn greet() { return 1; }`          ← declaration
    //   line 1: `struct greet { int x, }`            ← struct, same name
    //   line 2: `fn a() { return greet(); }`         ← caller 1
    //   line 3: `fn b() { return greet(); }`         ← caller 2
    //   line 4: `fn c() { return greet(); }`         ← caller 3
    //   line 5: `let _s = new greet { x: 0 };`       ← struct literal — NOT a call
    //
    // Cursor for the references request: on line 0 at char 3 (inside
    // "greet" on the fn declaration line).
    let src = concat!(
        "fn greet() { return 1; }\n",
        "struct greet { int x, }\n",
        "fn a() { return greet(); }\n",
        "fn b() { return greet(); }\n",
        "fn c() { return greet(); }\n",
        "let _s = new greet { x: 0 };\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///refs.rs","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- references WITHOUT includeDeclaration ----
    // Cursor on line 0 char 3 (inside `greet`).
    // Expected: 3 locations (one per caller); struct literal NOT included.
    let refs1 = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/references","params":{"textDocument":{"uri":"file:///refs.rs"},"position":{"line":0,"character":3},"context":{"includeDeclaration":false}}}"#;
    stdin.write_all(frame(refs1).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp1 = read_until_id(&mut stdout, 2, deadline).unwrap();

    // There should be exactly 3 Location entries — one for each of
    // lines 2, 3, 4 (the three fn bodies that call `greet()`).
    // Each Location serialises as `"uri"` + `"range"`. Count unique
    // "refs.rs" uri occurrences as a proxy for location count.
    let location_count = count_occurrences(&resp1, "refs.rs");
    assert_eq!(
        location_count, 3,
        "expected 3 call-site locations (no declaration), got {} in:\n{}",
        location_count, resp1,
    );

    // The struct literal line (line 5) must NOT appear.
    // Its line number in LSP is 5. We look for `"line":5` in the
    // response; if it's there, the struct literal snuck in.
    assert!(
        !resp1.contains("\"line\":5"),
        "struct literal line (5) must not appear in references:\n{}",
        resp1,
    );

    // ---- references WITH includeDeclaration ----
    // Same cursor, but `includeDeclaration: true` → 4 locations
    // (3 callers + the declaration on line 0).
    let refs2 = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/references","params":{"textDocument":{"uri":"file:///refs.rs"},"position":{"line":0,"character":3},"context":{"includeDeclaration":true}}}"#;
    stdin.write_all(frame(refs2).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp2 = read_until_id(&mut stdout, 3, deadline).unwrap();

    let location_count2 = count_occurrences(&resp2, "refs.rs");
    assert_eq!(
        location_count2, 4,
        "expected 4 locations (3 callers + declaration), got {} in:\n{}",
        location_count2, resp2,
    );
    // The declaration is on line 0; it should be present when
    // includeDeclaration is true.
    assert!(
        resp2.contains("\"line\":0"),
        "declaration line (0) should appear when includeDeclaration=true:\n{}",
        resp2,
    );

    // ---- references on a keyword cursor — should return null ----
    // Cursor on line 0 char 0 (`fn` keyword). `identifier_at` returns
    // None for keywords, so the handler returns null (no identifier
    // to look up).
    let refs3 = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/references","params":{"textDocument":{"uri":"file:///refs.rs"},"position":{"line":0,"character":0},"context":{"includeDeclaration":false}}}"#;
    stdin.write_all(frame(refs3).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp3 = read_until_id(&mut stdout, 4, deadline).unwrap();
    assert!(
        resp3.contains("\"result\":null"),
        "references on a keyword cursor should return null:\n{}",
        resp3,
    );

    // ---- shutdown ----
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
