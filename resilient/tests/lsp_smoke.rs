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
    env!("CARGO_BIN_EXE_rz")
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
    let init_body =
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
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
    stdin
        .write_all(frame(init).as_bytes())
        .expect("write initialize");
    stdin.flush().ok();

    let deadline = Instant::now() + Duration::from_secs(5);
    // Drain the initialize response (id=1).
    let _init_resp = read_one_message(&mut stdout, deadline).expect("read initialize response");

    // Step 2: initialized notification.
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin
        .write_all(frame(initialized).as_bytes())
        .expect("write initialized");

    // Step 3: didOpen with a 3-line program where line 3 is a known
    // type error. The typechecker rejects `let bad: int = "hi";`
    // (RES-053), and RES-080 prefixes the message with `<uri>:3:5:`.
    let did_open = r#"{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":"file:///tmp/lsp_test.rs","languageId":"resilient","version":1,"text":"let a = 1;\nlet b = 2;\nlet bad: int = \"hi\";"}}}"#;
    stdin
        .write_all(frame(did_open).as_bytes())
        .expect("write didOpen");
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
    let has_line_2 = diag_body.contains(r#""line":2"#) || diag_body.contains(r#""line": 2"#);
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
fn lsp_document_symbol_lists_outline() {
    // RES-185: round-trip through the real LSP server —
    // initialize → didOpen a 3-fn + 1-struct program →
    // textDocument/documentSymbol → assert the response lists
    // all four symbols.
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
    let init_deadline = Instant::now() + Duration::from_secs(5);
    let init_resp = read_one_message(&mut stdout, init_deadline).expect("read initialize response");
    // Capability smoke-check — the server should advertise
    // documentSymbolProvider.
    assert!(
        init_resp.contains(r#""documentSymbolProvider":true"#),
        "expected documentSymbolProvider:true in capabilities, got:\n{}",
        init_resp
    );

    // initialized
    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();

    // didOpen with a 3-fn + 1-struct program.
    let uri = "file:///tmp/lsp_docsym_test.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"fn alpha() {{ return 0; }}\nstruct Point {{ int x, int y }}\nfn beta(int n) {{ return n; }}\nfn gamma() {{ return 1; }}\n"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    // Drain the publishDiagnostics notification so it doesn't
    // land as the next read's body.
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    // textDocument/documentSymbol request.
    let docsym_req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/documentSymbol","params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
    );
    stdin.write_all(frame(&docsym_req).as_bytes()).unwrap();
    stdin.flush().ok();

    // Read the id:2 response specifically (skip any other
    // notifications the server might emit in the meantime).
    let response = read_until(
        &mut stdout,
        |body| body.contains(r#""id":2"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read documentSymbol response");

    // All four symbol names should appear in the response body.
    for name in ["alpha", "Point", "beta", "gamma"] {
        let needle = format!(r#""name":"{name}""#);
        assert!(
            response.contains(&needle),
            "expected `{needle}` in documentSymbol response:\n{response}"
        );
    }

    // Kind field — SymbolKind::FUNCTION is 12, STRUCT is 23 in
    // the LSP spec. Pin at least one of each to verify the
    // kinds are threading through.
    assert!(
        response.contains(r#""kind":12"#),
        "expected a FUNCTION kind (12) in: {response}"
    );
    assert!(
        response.contains(r#""kind":23"#),
        "expected a STRUCT kind (23) in: {response}"
    );

    // exit + clean shutdown.
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
fn lsp_workspace_symbol_searches_multiple_files() {
    // RES-186: pre-seed two .rs files in a temp workspace,
    // initialize with the workspace folder, issue a
    // `workspace/symbol` request, and assert BOTH files' symbols
    // come back.

    // Scratch workspace with two files.
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("res_186_smoke_{}_{}", std::process::id(), n));
    std::fs::create_dir_all(&root).expect("mkdir scratch");
    std::fs::write(
        root.join("mod_a.rs"),
        "fn a_fn() { return 0; }\nstruct A_Struct { int x }\n",
    )
    .unwrap();
    std::fs::write(root.join("mod_b.rs"), "fn b_fn() { return 0; }\n").unwrap();
    // URI of the scratch dir as a file:// URL.
    let root_uri = format!("file://{}", root.to_string_lossy().replace("\\", "/"));

    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // initialize with workspace_folders pointing at the scratch dir.
    let init = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"capabilities":{{}},"workspaceFolders":[{{"uri":"{root_uri}","name":"test"}}]}}}}"#
    );
    stdin.write_all(frame(&init).as_bytes()).unwrap();
    stdin.flush().ok();
    let init_deadline = Instant::now() + Duration::from_secs(5);
    let init_resp = read_one_message(&mut stdout, init_deadline).expect("read initialize response");
    assert!(
        init_resp.contains(r#""workspaceSymbolProvider":true"#),
        "expected workspaceSymbolProvider:true in capabilities, got:\n{}",
        init_resp
    );

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    // workspace/symbol with empty query — returns everything.
    let req = r#"{"jsonrpc":"2.0","id":2,"method":"workspace/symbol","params":{"query":""}}"#;
    stdin.write_all(frame(req).as_bytes()).unwrap();
    stdin.flush().ok();

    let response = read_until(
        &mut stdout,
        |body| body.contains(r#""id":2"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read workspace/symbol response");

    // All three symbol names should appear.
    for name in ["a_fn", "A_Struct", "b_fn"] {
        let needle = format!(r#""name":"{name}""#);
        assert!(
            response.contains(&needle),
            "expected `{needle}` in workspace/symbol response:\n{response}"
        );
    }
    // And each file's URI should appear.
    assert!(
        response.contains("mod_a.rs"),
        "expected mod_a.rs in response:\n{response}"
    );
    assert!(
        response.contains("mod_b.rs"),
        "expected mod_b.rs in response:\n{response}"
    );

    // Now a filtered query — "struct" should match only A_Struct.
    let req2 =
        r#"{"jsonrpc":"2.0","id":3,"method":"workspace/symbol","params":{"query":"struct"}}"#;
    stdin.write_all(frame(req2).as_bytes()).unwrap();
    stdin.flush().ok();
    let filtered = read_until(
        &mut stdout,
        |body| body.contains(r#""id":3"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read filtered response");
    assert!(
        filtered.contains(r#""name":"A_Struct""#),
        "filtered response should contain A_Struct:\n{filtered}"
    );
    assert!(
        !filtered.contains(r#""name":"a_fn""#) && !filtered.contains(r#""name":"b_fn""#),
        "filtered response should NOT contain *_fn names:\n{filtered}"
    );

    // Cleanup child process.
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

    // Cleanup scratch dir.
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn lsp_semantic_tokens_full() {
    // RES-187: initialize → didOpen a small program → request
    // `textDocument/semanticTokens/full` → assert the response
    // body is shaped like the LSP spec requires (a `data` array
    // whose length is a multiple of 5 and is non-empty for this
    // program).
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
    let init_deadline = Instant::now() + Duration::from_secs(5);
    let init_resp = read_one_message(&mut stdout, init_deadline).expect("read initialize response");
    // Capability advertised — presence of the `legend` key is
    // enough to confirm the server registered semantic tokens.
    assert!(
        init_resp.contains(r#""semanticTokensProvider""#),
        "expected semanticTokensProvider in capabilities, got:\n{}",
        init_resp
    );
    assert!(
        init_resp.contains(r#""legend""#),
        "expected legend in semanticTokensProvider, got:\n{}",
        init_resp
    );

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    // didOpen a program with each token type represented:
    // comment, keyword, fn decl, type decl, string, number,
    // variable, operator.
    let uri = "file:///tmp/lsp_semtok.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"// hi\nstruct Point {{ int x }}\nfn greet(int n) {{ let s = \"hi\"; let v = n + 1; return 0; }}\n"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    // Drain publishDiagnostics so the next read is the
    // semanticTokens response.
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    // textDocument/semanticTokens/full
    let req = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/semanticTokens/full","params":{{"textDocument":{{"uri":"{uri}"}}}}}}"#
    );
    stdin.write_all(frame(&req).as_bytes()).unwrap();
    stdin.flush().ok();

    let response = read_until(
        &mut stdout,
        |body| body.contains(r#""id":2"#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read semanticTokens response");

    // Shape: `{"jsonrpc":"2.0","id":2,"result":{"data":[...]}}`.
    assert!(
        response.contains(r#""data""#),
        "expected `data` field in semanticTokens response:\n{response}"
    );

    // Parse out the data array and verify:
    //  - length is a multiple of 5 (LSP spec);
    //  - length is > 0 (non-trivial program);
    //  - the first 5-tuple starts with `0,0,...` (first token on
    //    the first line at column 0 — the `//` comment).
    let data_start = response.find(r#""data":["#).expect("find data array");
    let after_bracket = data_start + r#""data":["#.len();
    let bracket_end = after_bracket + response[after_bracket..].find(']').expect("find closing ]");
    let nums: Vec<u32> = response[after_bracket..bracket_end]
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .collect();
    assert!(
        !nums.is_empty(),
        "data array should be non-empty, got response:\n{response}"
    );
    assert_eq!(
        nums.len() % 5,
        0,
        "data array must contain 5-tuples, got {} entries:\n{response}",
        nums.len()
    );
    // First token: the `//` comment at line 0 column 0.
    assert_eq!(nums[0], 0, "first deltaLine should be 0");
    assert_eq!(nums[1], 0, "first deltaStart should be 0");

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

#[test]
fn lsp_inlay_hint_types_for_unannotated_lets() {
    // RES-189 AC: 5-let snippet, 3 hints expected (the unannotated
    // ones). Round-trips through the real LSP server: initialize →
    // didOpen a program → textDocument/inlayHint over the whole
    // document → count `"kind":1` entries (TYPE kind = 1 per the
    // LSP spec).
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // initialize (no special init options; parameter hints stay off).
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().ok();
    let init_deadline = Instant::now() + Duration::from_secs(5);
    let init_resp = read_one_message(&mut stdout, init_deadline).expect("read initialize response");
    assert!(
        init_resp.contains(r#""inlayHintProvider""#),
        "expected inlayHintProvider in capabilities, got:\n{}",
        init_resp
    );

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    // 5 lets; 3 without annotation → 3 TYPE hints expected.
    let uri = "file:///tmp/lsp_inlay_test.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"fn main(int _d) {{ let a = 1; let b: int = 2; let c = true; let d: bool = false; let e = \"hi\"; return 0; }}\n"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().ok();

    // Drain publishDiagnostics so it doesn't land as the next read.
    let _ = read_until(
        &mut stdout,
        |body| body.contains(r#""method":"textDocument/publishDiagnostics""#),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read publishDiagnostics");

    // Whole-file range.
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

    // Count kind:1 entries (TYPE). 3 unannotated lets → 3 hints.
    // No parameter hints because the init didn't opt in.
    let type_hint_count = response.matches(r#""kind":1"#).count();
    assert_eq!(
        type_hint_count, 3,
        "expected 3 TYPE inlay hints, got {type_hint_count} in:\n{response}"
    );

    // Labels contain the inferred type names.
    assert!(
        response.contains(r#""label":": int""#),
        "expected `int` type-hint label in:\n{response}"
    );
    assert!(
        response.contains(r#""label":": bool""#),
        "expected `bool` type-hint label in:\n{response}"
    );
    assert!(
        response.contains(r#""label":": string""#),
        "expected `string` type-hint label in:\n{response}"
    );

    // No PARAMETER (kind 2) hints since the flag was not set.
    assert!(
        !response.contains(r#""kind":2"#),
        "expected NO parameter hints without opt-in, got:\n{response}"
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

#[test]
fn lsp_inlay_hint_parameter_hints_opt_in() {
    // When initialization_options sets
    // `resilient.inlayHints.parameters: true`, parameter hints
    // appear at user-fn call sites.
    let mut child = Command::new(bin())
        .arg("--lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn resilient --lsp");

    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // Opt in to parameter hints via the flat initialization option
    // form (clients typically send either flat or nested; we
    // accept both).
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{},"initializationOptions":{"resilient.inlayHints.parameters":true}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().ok();
    let _ = read_one_message(&mut stdout, Instant::now() + Duration::from_secs(5))
        .expect("read init response");

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().ok();

    let uri = "file:///tmp/lsp_inlay_param.rs";
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"fn add(int a, int b) {{ return a + b; }}\nfn main(int _d) {{ return add(1, 2); }}\nmain(0);\n"}}}}}}"#
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

    // Should see PARAMETER kind hints (kind = 2) now that we've
    // opted in.
    assert!(
        response.contains(r#""kind":2"#),
        "expected parameter hints under opt-in, got:\n{response}"
    );
    assert!(
        response.contains(r#""label":"a: ""#),
        "expected `a: ` param label in:\n{response}"
    );
    assert!(
        response.contains(r#""label":"b: ""#),
        "expected `b: ` param label in:\n{response}"
    );

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
    let body = read_until(
        &mut stdout,
        is_diag,
        Instant::now() + Duration::from_secs(5),
    )
    .expect("read clean publishDiagnostics");
    assert!(
        body.contains(r#""diagnostics":[]"#),
        "expected EMPTY diagnostics for clean program; got:\n{body}"
    );

    // Step 2: didChange to a buggy version (FULL sync = full text replace)
    let did_change_buggy = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didChange","params":{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":"let bad: int = \"hi\";"}}]}}}}"#
    );
    stdin
        .write_all(frame(&did_change_buggy).as_bytes())
        .unwrap();
    stdin.flush().ok();

    let body = read_until(
        &mut stdout,
        is_diag,
        Instant::now() + Duration::from_secs(5),
    )
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
    stdin
        .write_all(frame(&did_change_clean).as_bytes())
        .unwrap();
    stdin.flush().ok();

    let body = read_until(
        &mut stdout,
        is_diag,
        Instant::now() + Duration::from_secs(5),
    )
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
