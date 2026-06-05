//! LSP inlay-hint smoke tests for inferred function return types.
//!
//! Spawns `resilient --lsp`, opens a document with omitted return
//! annotations, and verifies the server surfaces TYPE inlay hints for
//! named functions and anonymous function literals. Also verifies the
//! type-hints initialization setting can disable those hints.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
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
        let n = r.read(&mut buf).map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF before LSP header complete".into());
        }
        header.push(buf[0]);
        if header.ends_with(b"\r\n\r\n") {
            let header_str =
                std::str::from_utf8(&header).map_err(|e| format!("bad header utf8: {e}"))?;
            for line in header_str.split("\r\n") {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("Content-Length:") {
                    content_length = Some(
                        rest.trim()
                            .parse::<usize>()
                            .map_err(|e| format!("bad Content-Length: {e}"))?,
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
            return Err(format!("timed out reading LSP body ({filled}/{len} bytes)"));
        }
        let n = r
            .read(&mut body[filled..])
            .map_err(|e| format!("body read error: {e}"))?;
        if n == 0 {
            return Err("unexpected EOF in LSP body".into());
        }
        filled += n;
    }
    String::from_utf8(body).map_err(|e| format!("bad body utf8: {e}"))
}

fn read_until<R: Read, F: Fn(&str) -> bool>(
    r: &mut R,
    pred: F,
    deadline: Instant,
) -> Result<String, String> {
    loop {
        let body = read_one_message(r, deadline)?;
        if pred(&body) {
            return Ok(body);
        }
    }
}

fn spawn_lsp() -> std::process::Child {
    Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp")
}

fn shutdown(mut child: std::process::Child, mut stdin: std::process::ChildStdin) {
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
            Err(e) => panic!("try_wait failed: {e}"),
        }
    }
}

#[test]
fn lsp_inlay_hint_types_include_omitted_function_returns() {
    let mut child = spawn_lsp();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().ok();
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""id":1"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read initialize response");

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    let uri = "file:///tmp/lsp_inlay_return.rs";
    let src = concat!(
        "fn answer() { return 42; }\n",
        "let add_one = fn(int x) { return x + 1; };\n",
    );
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"{src_escaped}"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    let req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":100,"character":0}}}}}}}}"#
    );
    stdin.write_all(frame(&req).as_bytes()).unwrap();
    stdin.flush().ok();

    let response = read_until(
        &mut stdout,
        |body| body.contains(r#""id":2"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read inlayHint response");

    let type_hint_count = response.matches(r#""kind":1"#).count();
    assert_eq!(
        response.matches(r#""label":" -> int""#).count(),
        2,
        "expected two inferred return labels, got:\n{response}"
    );
    assert_eq!(
        type_hint_count, 3,
        "expected TYPE hints for the named fn, closure return, and closure-valued let, got {type_hint_count} in:\n{response}"
    );

    shutdown(child, stdin);
}

#[test]
fn lsp_inlay_hint_type_setting_can_disable_type_hints() {
    let mut child = spawn_lsp();
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"initializationOptions":{"resilient.inlayHints.types":false}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().ok();
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""id":1"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read initialize response");

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    let uri = "file:///tmp/lsp_inlay_types_disabled.rs";
    let src = "fn answer() { return 42; }\nfn main(int _d) { let x = answer(); return x; }\n";
    let src_escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"{src_escaped}"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    let req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/inlayHint","params":{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":100,"character":0}}}}}}}}"#
    );
    stdin.write_all(frame(&req).as_bytes()).unwrap();
    stdin.flush().ok();

    let response = read_until(
        &mut stdout,
        |body| body.contains(r#""id":2"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read inlayHint response");

    assert!(
        !response.contains(r#""kind":1"#),
        "expected no TYPE hints when resilient.inlayHints.types=false, got:\n{response}"
    );

    shutdown(child, stdin);
}
