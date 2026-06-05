//! RES-2567: LSP find-references across workspace imports + variables.
//!
//! Adds coverage beyond RES-183's same-file function-only smoke test:
//! - cross-file function references via `use "..."`,
//! - cross-file struct-type references via `use "..."`,
//! - same-file variable declaration / read / write references.

#![cfg(feature = "lsp")]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
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

fn file_uri(path: &Path) -> String {
    format!("file://{}", path.to_string_lossy().replace('\\', "/"))
}

fn write_open(stdin: &mut impl Write, uri: &str, src: &str) {
    let escaped = src
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let did_open = format!(
        r#"{{"jsonrpc":"2.0","method":"textDocument/didOpen","params":{{"textDocument":{{"uri":"{uri}","languageId":"resilient","version":1,"text":"{escaped}"}}}}}}"#
    );
    stdin.write_all(frame(&did_open).as_bytes()).unwrap();
    stdin.flush().unwrap();
}

fn make_workspace(tag: &str) -> std::path::PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "res_2567_refs_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&root).expect("mkdir scratch workspace");
    root
}

#[test]
fn lsp_references_follow_workspace_imports_for_functions_and_structs() {
    let root = make_workspace("imports");
    let defs = root.join("defs.rz");
    let calls = root.join("calls.rz");
    let types = root.join("types.rz");

    std::fs::write(
        &defs,
        concat!(
            "pub fn greet() { return 1; }\n",
            "pub struct Point { int x, }\n",
        ),
    )
    .unwrap();
    std::fs::write(
        &calls,
        concat!(
            "use \"defs.rz\";\n",
            "fn a() { greet(); }\n",
            "fn b() { greet(); }\n",
        ),
    )
    .unwrap();
    std::fs::write(
        &types,
        concat!(
            "use \"defs.rz\";\n",
            "fn keep(Point p) -> Point {\n",
            "    let q: Point = p;\n",
            "    return new Point { x: 1 };\n",
            "}\n",
        ),
    )
    .unwrap();

    let root_uri = file_uri(&root);
    let defs_uri = file_uri(&defs);

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

    let init = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"capabilities":{{}},"workspaceFolders":[{{"uri":"{root_uri}","name":"refs"}}]}}}}"#
    );
    stdin.write_all(frame(&init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let init_resp = read_until_id(&mut stdout, 1, deadline).unwrap();
    assert!(
        init_resp.contains("\"referencesProvider\""),
        "expected referencesProvider in initialize response:\n{}",
        init_resp
    );

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().unwrap();

    write_open(
        &mut stdin,
        &defs_uri,
        &std::fs::read_to_string(&defs).expect("read defs"),
    );

    let fn_refs = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/references","params":{{"textDocument":{{"uri":"{defs_uri}"}},"position":{{"line":0,"character":8}},"context":{{"includeDeclaration":true}}}}}}"#
    );
    stdin.write_all(frame(&fn_refs).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let fn_resp = read_until_id(&mut stdout, 2, deadline).unwrap();
    assert!(
        fn_resp.contains("defs.rz"),
        "function refs should include declaration file:\n{}",
        fn_resp
    );
    assert!(
        fn_resp.contains("calls.rz"),
        "function refs should include imported call sites:\n{}",
        fn_resp
    );
    assert!(
        fn_resp.contains("\"line\":1"),
        "function refs should include first imported caller line:\n{}",
        fn_resp
    );
    assert!(
        fn_resp.contains("\"line\":2"),
        "function refs should include second imported caller line:\n{}",
        fn_resp
    );

    let struct_refs = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/references","params":{{"textDocument":{{"uri":"{defs_uri}"}},"position":{{"line":1,"character":11}},"context":{{"includeDeclaration":true}}}}}}"#
    );
    stdin.write_all(frame(&struct_refs).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let struct_resp = read_until_id(&mut stdout, 3, deadline).unwrap();
    assert!(
        struct_resp.contains("defs.rz"),
        "struct refs should include declaration file:\n{}",
        struct_resp
    );
    assert!(
        struct_resp.contains("types.rz"),
        "struct refs should include imported type uses:\n{}",
        struct_resp
    );
    for line in [1_u32, 2, 3] {
        assert!(
            struct_resp.contains(&format!("\"line\":{line}")),
            "struct refs should include types.rz line {line}:\n{}",
            struct_resp
        );
    }

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn lsp_references_include_variable_decl_reads_and_writes() {
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

    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}"#;
    stdin.write_all(frame(init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let _ = read_until_id(&mut stdout, 1, deadline).unwrap();

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let src = concat!(
        "fn main() {\n",
        "    let counter = 0;\n",
        "    counter = counter + 1;\n",
        "    let done = counter;\n",
        "}\n",
    );
    write_open(&mut stdin, "file:///vars.rz", src);

    let refs = r#"{"jsonrpc":"2.0","id":2,"method":"textDocument/references","params":{"textDocument":{"uri":"file:///vars.rz"},"position":{"line":1,"character":9},"context":{"includeDeclaration":true}}}"#;
    stdin.write_all(frame(refs).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let resp = read_until_id(&mut stdout, 2, deadline).unwrap();

    for line in [1_u32, 2, 3] {
        assert!(
            resp.contains(&format!("\"line\":{line}")),
            "variable refs should include line {line}:\n{}",
            resp
        );
    }
    let location_count = resp.matches("vars.rz").count();
    assert!(
        location_count >= 4,
        "expected declaration + write + reads for counter, got {} in:\n{}",
        location_count,
        resp
    );

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
