//! RES-182a: LSP go-to-definition integration smoke test.
//!
//! Spawns `resilient --lsp`, opens a document with one `fn` and
//! one `struct` decl, then drives `textDocument/definition` at
//! (a) a call-site reference to the fn and (b) a reference to
//! the struct. Asserts each response carries a `Location`
//! pointing back at the corresponding decl line.
//!
//! Mirrors `lsp_smoke.rs` / `lsp_hover_smoke.rs` framing so the
//! test dep tree stays empty. Gated on `--features lsp`.

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
fn lsp_goto_definition_returns_location_for_top_level_decls() {
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
        init_resp.contains("\"definitionProvider\""),
        "expected `definitionProvider` in initialize response, got:\n{}",
        init_resp,
    );

    // ---- initialized ----
    let init_done = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(init_done).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // ---- didOpen a document ----
    // Layout:
    //   line 0: `fn add(int a, int b) -> int { return a + b; }`
    //   line 1: `struct Point { int x, int y, }`
    //   line 2: `let r = add(1, 2);`     <-- cursor here → should jump to line 0
    //   line 3: `let p = new Point { x: 0, y: 0 };` <-- cursor → line 1
    let src = concat!(
        "fn add(int a, int b) -> int { return a + b; }\n",
        "struct Point { int x, int y, }\n",
        "let r = add(1, 2);\n",
        "let p = new Point { x: 0, y: 0 };\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"file:///goto.rs","languageId":"resilient","version":1,"text":"{}"}}}}}}"#,
        src_escaped,
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();

    // Cursor on the `a` of `add(1, 2)` at line 2, char 8
    // (`let r = add(...)` — positions: l=0,e=1,t=2, =3,r=4, =5,==6, =7,a=8).
    let def1 = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///goto.rs"},"position":{"line":2,"character":8}}}"#;
    stdin.write_all(frame(def1).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp1 = read_until_id(&mut stdout, 2, deadline).unwrap();
    // The response should contain a Location with line 0 (the
    // `fn add` decl). tower-lsp serializes GotoDefinitionResponse::
    // Scalar as a single Location.
    assert!(
        resp1.contains("\"line\":0"),
        "goto for `add` didn't return line 0:\n{}",
        resp1,
    );
    assert!(
        resp1.contains("goto.rs"),
        "goto response missing document uri:\n{}",
        resp1,
    );

    // Cursor on `Point` in `new Point { ... }` — line 3.
    // `let p = new Point { ... }`
    // positions: l=0,e=1,t=2, =3,p=4, =5,==6, =7,n=8,e=9,w=10, =11,P=12
    let def2 = r#"{"jsonrpc":"2.0","id":3,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///goto.rs"},"position":{"line":3,"character":13}}}"#;
    stdin.write_all(frame(def2).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp2 = read_until_id(&mut stdout, 3, deadline).unwrap();
    assert!(
        resp2.contains("\"line\":1"),
        "goto for `Point` didn't return line 1:\n{}",
        resp2,
    );

    // Cursor on a keyword — should return null (no jump).
    let def3 = r#"{"jsonrpc":"2.0","id":4,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///goto.rs"},"position":{"line":0,"character":0}}}"#;
    stdin.write_all(frame(def3).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp3 = read_until_id(&mut stdout, 4, deadline).unwrap();
    assert!(
        resp3.contains("\"result\":null"),
        "expected null for keyword position:\n{}",
        resp3,
    );

    // Cursor on a LOCAL (the `r` binder on line 2) — RES-182a
    // only handles top-level decls, so this should return null
    // too. This pins the scope: when RES-182b lands local
    // resolution, this assertion will need to change.
    let def4 = r#"{"jsonrpc":"2.0","id":5,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///goto.rs"},"position":{"line":2,"character":4}}}"#;
    stdin.write_all(frame(def4).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp4 = read_until_id(&mut stdout, 5, deadline).unwrap();
    assert!(
        resp4.contains("\"result\":null"),
        "expected null for local binder `r` (RES-182a doesn't handle locals):\n{}",
        resp4,
    );

    // ---- shutdown ----
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
