//! RES-338: LSP rename-symbol integration smoke test.
//!
//! Spawns `resilient --lsp`, opens a document with a function `foo`
//! declared once and called twice, then drives `textDocument/rename`
//! at the cursor on the declaration to rename `foo` → `bar`.
//! Asserts the response is a `WorkspaceEdit` covering the declaration
//! site and both call sites.  Also verifies:
//!   - `renameProvider` is advertised in `initialize`.
//!   - Renaming to an invalid identifier returns a JSON-RPC error.
//!   - Renaming to a name that already exists returns a JSON-RPC error.
//!   - `textDocument/prepareRename` returns a range for a renamable symbol.
//!
//! Mirrors `lsp_references_smoke.rs` / `lsp_goto_def_smoke.rs` framing
//! so the test dep tree stays empty.  Gated on `--features lsp`.

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
fn lsp_rename_foo_to_bar_returns_workspace_edit() {
    // Layout (0-indexed LSP lines):
    //   line 0: `fn foo(int x) -> int { return x; }`  ← declaration
    //   line 1: `let a = foo(1);`                      ← call site 1
    //   line 2: `let b = foo(2);`                      ← call site 2
    //
    // Cursor for prepareRename and rename: line 0, character 3
    // (the `f` in `foo` on the declaration line).
    // Expected: WorkspaceEdit with 3 TextEdits (1 decl + 2 calls),
    // all replacing `foo` with `bar`.

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
        init_resp.contains("\"renameProvider\""),
        "expected `renameProvider` in initialize response, got:\n{}",
        init_resp,
    );

    // ---- initialized ----
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen a document ----
    let src = concat!(
        "fn foo(int x) -> int { return x; }\n",
        "let a = foo(1);\n",
        "let b = foo(2);\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///rename_test.rz","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- prepareRename on the `foo` declaration (line 0, char 3) ----
    // Should return a RangeWithPlaceholder covering `foo`.
    let prepare = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/prepareRename","params":{"textDocument":{"uri":"file:///rename_test.rz"},"position":{"line":0,"character":3}}}"#;
    stdin.write_all(frame(prepare).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let prepare_resp = read_until_id(&mut stdout, 2, deadline).unwrap();
    assert!(
        !prepare_resp.contains("\"result\":null"),
        "prepareRename should not return null for a renamable fn name:\n{}",
        prepare_resp,
    );
    assert!(
        prepare_resp.contains("\"placeholder\":\"foo\""),
        "prepareRename should return placeholder `foo`:\n{}",
        prepare_resp,
    );

    // ---- rename `foo` → `bar` from the declaration site ----
    let rename1 = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///rename_test.rz"},"position":{"line":0,"character":3},"newName":"bar"}}"#;
    stdin.write_all(frame(rename1).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let rename_resp = read_until_id(&mut stdout, 3, deadline).unwrap();

    // The response must contain a WorkspaceEdit (not null, not error).
    assert!(
        !rename_resp.contains("\"error\""),
        "rename should not return an error:\n{}",
        rename_resp,
    );
    assert!(
        !rename_resp.contains("\"result\":null"),
        "rename should return a WorkspaceEdit, not null:\n{}",
        rename_resp,
    );
    // The new name must appear in the edits.
    assert!(
        rename_resp.contains("\"newText\":\"bar\""),
        "rename response should contain newText `bar`:\n{}",
        rename_resp,
    );
    // Exactly 3 edit sites: declaration (line 0) + 2 call sites (lines 1, 2).
    // Each TextEdit contains `"newText"`, so we count those.
    let edit_count = count_occurrences(&rename_resp, "\"newText\":\"bar\"");
    assert_eq!(
        edit_count, 3,
        "expected 3 edits (1 decl + 2 calls), got {} in:\n{}",
        edit_count, rename_resp,
    );
    // Verify the declaration site (line 0) is present.
    assert!(
        rename_resp.contains("\"line\":0"),
        "rename should include the declaration on line 0:\n{}",
        rename_resp,
    );
    // Verify both call-site lines are present.
    assert!(
        rename_resp.contains("\"line\":1"),
        "rename should include call site on line 1:\n{}",
        rename_resp,
    );
    assert!(
        rename_resp.contains("\"line\":2"),
        "rename should include call site on line 2:\n{}",
        rename_resp,
    );

    // ---- rename from a call site (line 1, char 8) — same result ----
    let rename2 = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///rename_test.rz"},"position":{"line":1,"character":8},"newName":"bar"}}"#;
    stdin.write_all(frame(rename2).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let rename_resp2 = read_until_id(&mut stdout, 4, deadline).unwrap();
    assert!(
        rename_resp2.contains("\"newText\":\"bar\""),
        "rename from call site should also work:\n{}",
        rename_resp2,
    );
    let edit_count2 = count_occurrences(&rename_resp2, "\"newText\":\"bar\"");
    assert_eq!(
        edit_count2, 3,
        "expected 3 edits from call-site cursor, got {} in:\n{}",
        edit_count2, rename_resp2,
    );

    // ---- rename to an invalid identifier — must return error ----
    let rename_invalid = r#"{"jsonrpc":"2.0","id":5,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///rename_test.rz"},"position":{"line":0,"character":3},"newName":"123invalid"}}"#;
    stdin.write_all(frame(rename_invalid).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let invalid_resp = read_until_id(&mut stdout, 5, deadline).unwrap();
    assert!(
        invalid_resp.contains("\"error\""),
        "rename to invalid identifier should return an error:\n{}",
        invalid_resp,
    );

    // ---- shutdown ----
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn lsp_rename_conflict_returns_error() {
    // Layout:
    //   line 0: `fn foo() -> int { return 1; }`
    //   line 1: `fn bar() -> int { return 2; }`
    //
    // Renaming `foo` to `bar` must fail because `bar` is already
    // a top-level binding.

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

    // initialize
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let _ = read_until_id(&mut stdout, 1, deadline).unwrap();

    // initialized
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // didOpen — two top-level functions
    let src = concat!(
        "fn foo() -> int { return 1; }\n",
        "fn bar() -> int { return 2; }\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///conflict_test.rz","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // rename `foo` → `bar` — must return an error (name collision)
    let rename = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/rename","params":{"textDocument":{"uri":"file:///conflict_test.rz"},"position":{"line":0,"character":3},"newName":"bar"}}"#;
    stdin.write_all(frame(rename).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp = read_until_id(&mut stdout, 2, deadline).unwrap();
    assert!(
        resp.contains("\"error\""),
        "rename to conflicting name `bar` must return an error:\n{}",
        resp,
    );
    assert!(
        resp.contains("bar"),
        "error message should mention the conflicting name:\n{}",
        resp,
    );

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
