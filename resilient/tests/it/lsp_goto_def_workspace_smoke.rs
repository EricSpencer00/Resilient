//! RES-3135: LSP go-to-definition across on-disk workspace imports.
//!
//! Opens only the importing file, then asks for definitions of imported
//! symbols. The response should point at the unopened `defs.rz` file.

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
        "res_3135_goto_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&root).expect("mkdir scratch workspace");
    root
}

#[test]
fn lsp_goto_definition_follows_workspace_imports_for_unopened_files() {
    let root = make_workspace("imports");
    let defs = root.join("defs.rz");
    let main = root.join("main.rz");

    std::fs::write(
        &defs,
        concat!(
            "pub fn greet() { return 1; }\n",
            "pub struct Point { int x, }\n",
        ),
    )
    .unwrap();
    std::fs::write(
        &main,
        concat!(
            "use \"defs.rz\";\n",
            "fn run() {\n",
            "    let answer = greet();\n",
            "    let point = new Point { x: 1 };\n",
            "}\n",
        ),
    )
    .unwrap();

    let root_uri = file_uri(&root);
    let main_uri = file_uri(&main);

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
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"capabilities":{{}},"workspaceFolders":[{{"uri":"{root_uri}","name":"goto"}}]}}}}"#
    );
    stdin.write_all(frame(&init).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let init_resp = read_until_id(&mut stdout, 1, deadline).unwrap();
    assert!(
        init_resp.contains("\"definitionProvider\""),
        "expected definitionProvider in initialize response:\n{}",
        init_resp
    );

    let initialized = r#"{"jsonrpc":"2.0","method":"initialized","params":{}}"#;
    stdin.write_all(frame(initialized).as_bytes()).unwrap();
    stdin.flush().unwrap();

    write_open(
        &mut stdin,
        &main_uri,
        &std::fs::read_to_string(&main).expect("read main"),
    );

    let fn_def = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{{"textDocument":{{"uri":"{main_uri}"}},"position":{{"line":2,"character":17}}}}}}"#
    );
    stdin.write_all(frame(&fn_def).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let fn_resp = read_until_id(&mut stdout, 2, deadline).unwrap();
    assert!(
        fn_resp.contains("defs.rz"),
        "function definition should resolve to imported file:\n{}",
        fn_resp
    );
    assert!(
        fn_resp.contains("\"line\":0"),
        "function definition should point at greet declaration:\n{}",
        fn_resp
    );

    let struct_def = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"textDocument/definition","params":{{"textDocument":{{"uri":"{main_uri}"}},"position":{{"line":3,"character":20}}}}}}"#
    );
    stdin.write_all(frame(&struct_def).as_bytes()).unwrap();
    stdin.flush().unwrap();
    let struct_resp = read_until_id(&mut stdout, 3, deadline).unwrap();
    assert!(
        struct_resp.contains("defs.rz"),
        "struct definition should resolve to imported file:\n{}",
        struct_resp
    );
    assert!(
        struct_resp.contains("\"line\":1"),
        "struct definition should point at Point declaration:\n{}",
        struct_resp
    );

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}
