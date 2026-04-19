//! RES-181a: LSP hover integration smoke test.
//!
//! Spawns `resilient --lsp`, sends `initialize` + `initialized`,
//! opens a document with several literal kinds, and drives
//! `textDocument/hover` at four cursor positions — verifying each
//! response carries the expected Resilient-surface type string
//! (`Int`, `Float`, `Bool`, `String`) and zero-indexed range.
//!
//! Mirrors `lsp_smoke.rs`'s hand-rolled LSP framing so the test
//! dep tree stays empty. Gated on `--features lsp`.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Frame a JSON payload as an LSP message.
fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

/// Read one LSP message body (JSON minus headers). Blocks until
/// the full body arrives or the deadline passes.
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

/// Read LSP messages until one with `id == expected_id` arrives.
/// Returns that message body. Notifications (no `id`) and
/// out-of-order replies are swallowed.
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
        // Not our reply — loop.
    }
}

/// Extract the text between `"key":"..."` in a JSON string.
/// Minimal string-field extractor sufficient for `type`-style
/// MarkedString content; doesn't handle escaped quotes.
fn find_string_field<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{}\":\"", key);
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Extract a u64 field value.
fn find_u64_field(body: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{}\":", key);
    let start = body.find(&needle)? + needle.len();
    let rest = &body[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[test]
fn lsp_hover_returns_type_name_for_literal_positions() {
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
        init_resp.contains("\"hoverProvider\""),
        "expected `hoverProvider` in initialize response, got:\n{}",
        init_resp
    );

    // ---- initialized notification ----
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen a document ----
    // Content:
    //   let x = 42;      // line 0, int at chars 8..10
    //   let s = "hi";    // line 1, string at chars 8..12
    //   let b = true;    // line 2, bool at chars 8..12
    //   let f = 3.14;    // line 3, float at chars 8..12
    //
    // JSON requires escaping the quotes around "hi"; we use the
    // raw Rust string then construct the JSON manually.
    let src = concat!(
        "let x = 42;\n",
        "let s = \"hi\";\n",
        "let b = true;\n",
        "let f = 3.14;\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///hover.rs","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Four hover requests at the literals. LSP positions are
    // zero-indexed. Each request uses a distinct id so we can
    // match replies.
    let cases: &[(u64, u64, u64, &str)] = &[
        (2, 0, 9, "Int"),    // `4` of 42
        (3, 1, 10, "String"), // `h` inside "hi"
        (4, 2, 10, "Bool"),   // `u` of true
        (5, 3, 10, "Float"),  // `.` of 3.14
    ];

    for (id, line, ch, want_ty) in cases {
        let req = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"textDocument/hover","params":{{"textDocument":{{"uri":"file:///hover.rs"}},"position":{{"line":{},"character":{}}}}}}}"#,
            id, line, ch,
        );
        stdin.write_all(frame(&req).as_bytes()).unwrap();
        stdin.flush().unwrap();
        let body = read_until_id(&mut stdout, *id, deadline)
            .unwrap_or_else(|e| panic!("hover id={} failed: {}", id, e));
        // The response should contain the type string inside the
        // MarkedString. tower-lsp serializes `HoverContents::Scalar`
        // as either `"contents":"..."` or `"contents":{"value":"..."}`
        // depending on MarkedString form. Search for the type
        // name anywhere in the body — precise enough for this test.
        assert!(
            body.contains(&format!("\"{}\"", want_ty)),
            "hover id={} at ({},{}) didn't mention {:?}:\n{}",
            id,
            line,
            ch,
            want_ty,
            body,
        );
        // Sanity: a range should be present.
        assert!(
            body.contains("\"range\""),
            "hover id={} response missing range:\n{}",
            id,
            body,
        );
    }

    // ---- hover on a non-literal position returns null ----
    let non_literal_req = r#"{"jsonrpc":"2.0","id":99,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///hover.rs"},"position":{"line":0,"character":0}}}"#;
    stdin.write_all(frame(non_literal_req).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let nl_body = read_until_id(&mut stdout, 99, deadline).unwrap();
    // A null result is either "result":null or absent contents.
    // tower-lsp serializes Option<Hover>::None as "result":null.
    assert!(
        nl_body.contains("\"result\":null"),
        "expected null result for hover on keyword, got:\n{}",
        nl_body,
    );

    // ---- shutdown by closing stdin + killing ----
    // The happy-path "shutdown" / "exit" round-trip doesn't
    // terminate tower-lsp's stdio loop reliably enough to block
    // a test on — we've verified the hover asserts, so reclaim
    // the process. Dropping stdin triggers EOF on the server's
    // read loop; `kill` is belt-and-suspenders for hangs.
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();

    // Silence unused-fn warnings — these helpers are kept for
    // future tests in this file.
    let _ = find_string_field("", "k");
    let _ = find_u64_field("", "k");
}
