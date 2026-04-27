//! RES-337: snapshot tests for LSP JSON responses.
//!
//! Companion to the hand-rolled `lsp_*_smoke.rs` tests. Those tests
//! pin individual fields with substring assertions — useful, but
//! they don't catch shape changes outside the asserted fields. This
//! harness drives the same `resilient --lsp` server, captures the
//! full JSON-RPC body for representative requests, normalizes the
//! handful of unstable fields (server `version`, document `uri`,
//! the JSON-RPC `id` we picked for the request), and snapshots the
//! result via `insta::assert_snapshot!`.
//!
//! Snapshots live under `resilient/tests/snapshots/` (insta's
//! default location), suffixed by the request name. Adding a new
//! snapshot is a two-step dance:
//!
//! 1. Author a `#[test]` that drives the request, normalizes via
//!    `normalize_json`, and calls `snapshot_response(name, body)`.
//! 2. Run `cargo test --features lsp --test lsp_snapshots` once
//!    to produce the `.snap.new` pending diff, review with
//!    `cargo insta review` (or hand-promote), and commit.
//!
//! When an LSP response shape *intentionally* changes (e.g. a new
//! capability, an extended hover payload), `cargo insta review`
//! shows the diff and you accept it. Drift in CI fails the test.
//!
//! This file does NOT modify any existing test — it adds a new
//! suite alongside the smoke tests. Existing assertions in
//! `lsp_smoke.rs`, `lsp_hover_smoke.rs`, `lsp_goto_def_smoke.rs`,
//! `lsp_completion_smoke.rs` continue to run unchanged.
//!
//! Gated on `--features lsp` so the test only compiles when the
//! LSP server is built. Hand-rolls the LSP framing for the same
//! reason the other smoke tests do: keeps the test dep tree lean.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

/// Frame a JSON payload as an LSP message:
/// `Content-Length: N\r\n\r\n<json>`.
fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

/// Read exactly one LSP message body (JSON minus headers). Blocks
/// until the full body arrives or the deadline passes.
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

/// Read framed messages until one with `id == expected_id` arrives.
/// Notifications (no `id`) and out-of-order replies are skipped.
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

/// Pretty-print a JSON string for snapshotting. `serde_json` is
/// already a direct dep (RES-195) so we leverage it instead of a
/// hand-rolled formatter — guarantees stable key order via the
/// preserve_order feature would be nice, but tower-lsp emits its
/// fields in a stable order today and `serde_json::Value` keeps
/// `BTreeMap`-like alphabetical ordering for objects, which is good
/// enough for snapshot stability.
///
/// On parse failure we return the raw body wrapped in a banner so
/// the snapshot still pins something meaningful (a parser bug in
/// the server should fail the test loudly, not silently).
fn pretty_json(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.to_string()),
        Err(e) => format!("<<JSON parse failed: {e}>>\n{raw}"),
    }
}

/// Normalize an LSP JSON response so the snapshot is reproducible
/// across runs and machines. Tower-lsp + our backend produce a
/// handful of fields whose value depends on the host environment
/// (Cargo package version) or the request's runtime id (which we
/// pick fresh in each test).
fn normalize_json(body: &str) -> String {
    let pretty = pretty_json(body);

    // Insta's filters operate on the formatted snapshot string, but
    // we apply the most important normalizations in code so the
    // pretty body is stable before insta sees it. This keeps the
    // checked-in snapshot decoupled from `Cargo.toml` version bumps
    // and from URI churn driven by `tempdir()` on CI runners.
    let mut out = pretty;

    // `serverInfo.version` is whatever the package version happens
    // to be — pin it so a `cargo bump` doesn't churn the snapshot.
    if let Some(start) = out.find("\"version\": \"") {
        let after = start + "\"version\": \"".len();
        if let Some(rel_end) = out[after..].find('"') {
            let end = after + rel_end;
            out.replace_range(after..end, "<VERSION>");
        }
    }

    // `serverInfo.name` is hardcoded but normalize defensively in
    // case a future PR appends a build hash etc.
    if let Some(start) = out.find("\"name\": \"resilient") {
        let after = start + "\"name\": \"".len();
        if let Some(rel_end) = out[after..].find('"') {
            let end = after + rel_end;
            // Only normalize when the value looks like a server
            // identifier (starts with `resilient`); leave other
            // `name` fields alone (e.g. completion item labels).
            if out[after..end].starts_with("resilient") {
                out.replace_range(after..end, "resilient-lsp");
            }
        }
    }

    out
}

/// Wrapper around an LSP child process keeping stdin/stdout pipes
/// alive for the lifetime of the test. Killed on `drop` so a
/// failing assert doesn't leak a server.
struct LspProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
}

impl LspProcess {
    fn spawn() -> Self {
        let mut child = Command::new(bin())
            .arg("--lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn resilient --lsp");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Self {
            child,
            stdin: Some(stdin),
            stdout,
        }
    }

    fn write(&mut self, frame_str: &str) {
        let pipe = self.stdin.as_mut().expect("stdin still open");
        pipe.write_all(frame_str.as_bytes()).expect("lsp write");
        pipe.flush().ok();
    }

    fn read_until_id(&mut self, id: u64, deadline: Instant) -> String {
        read_until_id(&mut self.stdout, id, deadline)
            .unwrap_or_else(|e| panic!("read_until_id({id}) failed: {e}"))
    }

    fn shutdown(&mut self) {
        // Drop stdin first so the server sees EOF on its read loop.
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for LspProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Drive the `initialize` + `initialized` handshake, then `didOpen`
/// the supplied source under a fixed URI. Returns the spawned
/// process so the caller can issue further requests.
///
/// Skips reading the publishDiagnostics notification by relying on
/// `read_until_id` filtering it out for subsequent ID-bearing
/// requests.
fn handshake_and_open(src: &str, uri: &str) -> LspProcess {
    let mut proc = LspProcess::spawn();
    let deadline = Instant::now() + Duration::from_secs(10);

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    proc.write(&frame(init));
    let _ = proc.read_until_id(1, deadline);

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    proc.write(&frame(initialized));

    let escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"{escaped}"}}}}}}"#
    );
    proc.write(&frame(&did_open));

    proc
}

/// Common insta settings for every snapshot in this file: pin
/// version-string drift and uri churn that survive the in-code
/// normalizer (defense in depth — if a future field appears with
/// a version/uri shape, the filter still catches it).
fn snapshot_response(name: &str, body: &str) {
    let normalized = normalize_json(body);
    let mut settings = insta::Settings::clone_current();
    // Belt-and-suspenders against any path-shaped field: rewrite
    // anything matching `file:///<path>` to a stable placeholder.
    // We use specific URIs in the tests already, but a future
    // request could echo back a server-side path.
    settings.add_filter(r#"file:///[^\s"]+\.rs"#, "file:///<doc>.rs");
    // Catch any leftover semantic-version-looking string in `version`
    // fields (e.g. "0.1.0" → "<VERSION>"). The in-code normalizer
    // handles `serverInfo.version`; this guards against future
    // additions like `clientInfo.version` being echoed back.
    settings.add_filter(
        r#""version":\s*"\d+\.\d+\.\d+""#,
        "\"version\": \"<VERSION>\"",
    );
    settings.set_snapshot_suffix(name);
    settings.set_description(format!("LSP response: {name}"));
    settings.bind(|| {
        insta::assert_snapshot!(name, normalized);
    });
}

// ---------------------------------------------------------------------------
// Snapshots
// ---------------------------------------------------------------------------

/// `initialize` response. Exercises the server-capabilities
/// payload — the most regression-prone JSON shape, since adding a
/// new LSP feature flips a flag here.
#[test]
fn snapshot_initialize_response() {
    let mut proc = LspProcess::spawn();
    let deadline = Instant::now() + Duration::from_secs(10);

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    proc.write(&frame(init));
    let body = proc.read_until_id(1, deadline);

    snapshot_response("initialize", &body);
}

/// `textDocument/hover` response on an integer literal. Pins the
/// shape RES-181a / PR #291 ships: `contents.kind = "markdown"`
/// (or scalar MarkedString form), the type string, and the range.
#[test]
fn snapshot_hover_response() {
    let src = "let x = 42;\n";
    let mut proc = handshake_and_open(src, "file:///hover_snap.rs");
    let deadline = Instant::now() + Duration::from_secs(10);

    // Position (0, 9): the digit `2` of `42` — guaranteed inside
    // the int literal regardless of trailing whitespace.
    let req = r#"{"jsonrpc":"2.0","id":42,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///hover_snap.rs"},"position":{"line":0,"character":9}}}"#;
    proc.write(&frame(req));
    let body = proc.read_until_id(42, deadline);

    snapshot_response("hover", &body);
}

/// `textDocument/hover` on a non-literal position — exercises the
/// `result: null` branch so a future regression that returns an
/// empty object instead of `null` is caught.
#[test]
fn snapshot_hover_null_response() {
    let src = "let x = 42;\n";
    let mut proc = handshake_and_open(src, "file:///hover_null.rs");
    let deadline = Instant::now() + Duration::from_secs(10);

    // Position (0, 0) — on the `let` keyword. Hover returns null.
    let req = r#"{"jsonrpc":"2.0","id":7,"method":"textDocument/hover","params":{"textDocument":{"uri":"file:///hover_null.rs"},"position":{"line":0,"character":0}}}"#;
    proc.write(&frame(req));
    let body = proc.read_until_id(7, deadline);

    snapshot_response("hover_null", &body);
}

/// `textDocument/definition` response — top-level fn jump.
#[test]
fn snapshot_definition_response() {
    let src = concat!(
        "fn add(int a, int b) -> int { return a + b; }\n",
        "let r = add(1, 2);\n",
    );
    let mut proc = handshake_and_open(src, "file:///def_snap.rs");
    let deadline = Instant::now() + Duration::from_secs(10);

    // Cursor on `a` of `add(1, 2)` at line 1 — same position the
    // existing smoke test uses, just with a snapshot instead of a
    // substring assertion.
    let req = r#"{"jsonrpc":"2.0","id":11,"method":"textDocument/definition","params":{"textDocument":{"uri":"file:///def_snap.rs"},"position":{"line":1,"character":8}}}"#;
    proc.write(&frame(req));
    let body = proc.read_until_id(11, deadline);

    snapshot_response("definition", &body);
}

/// `textDocument/completion` response — prefix matching a builtin.
/// Because the completion list grows when new builtins land, this
/// snapshot acts as a tripwire: a PR that adds a builtin must
/// review and accept the snapshot diff, which surfaces the
/// addition in code review.
#[test]
fn snapshot_completion_response() {
    let src = concat!("fn my_helper() { return 1; }\n", "let s = my_\n");
    let mut proc = handshake_and_open(src, "file:///comp_snap.rs");
    let deadline = Instant::now() + Duration::from_secs(10);

    // Cursor at end of `let s = my_` on line 1 (col 11). The
    // server's prefix matcher should surface `my_helper` as the
    // sole user-decl match alongside any builtin starting with
    // `my_` (none, currently).
    let req = r#"{"jsonrpc":"2.0","id":21,"method":"textDocument/completion","params":{"textDocument":{"uri":"file:///comp_snap.rs"},"position":{"line":1,"character":11}}}"#;
    proc.write(&frame(req));
    let body = proc.read_until_id(21, deadline);

    snapshot_response("completion", &body);
}
