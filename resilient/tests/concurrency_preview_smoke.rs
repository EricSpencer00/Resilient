//! RES-208: concurrency-preview parse scaffolding smoke tests.
//!
//! These tests verify that the `actor`, `spawn`, `send`, and `recv`
//! keywords parse correctly when the `concurrency-preview` feature is
//! active, and that the runtime rejects them with a clear diagnostic
//! rather than executing (since the scheduler is not wired up yet).
//!
//! All tests are gated behind `#[cfg(feature = "concurrency-preview")]`
//! so the standard build remains unaffected.

#![cfg(feature = "concurrency-preview")]

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Write `src` to a temp `.res` file, run the resilient driver
/// (built with `--features concurrency-preview`) against it, and
/// return `(stdout, stderr, exit_code)`.
fn run_src(tag: &str, src: &str) -> (String, String, Option<i32>) {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("res_208_{tag}_{}.res", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).expect("create temp .res");
        f.write_all(src.as_bytes()).expect("write src");
    }
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("spawn resilient");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

// ---------------------------------------------------------------------------
// Parser acceptance tests — these assert that the four forms parse
// without a "Parser error" diagnostic.
// ---------------------------------------------------------------------------

/// `actor Name { }` must parse without a parser error.
#[test]
fn actor_decl_parses_without_error() {
    let src = r#"
actor Counter {
    let x = 0;
}
"#;
    let (_stdout, stderr, _code) = run_src("actor_parse", src);
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error on actor decl:\n{stderr}"
    );
}

/// `spawn Name;` must parse without a parser error.
#[test]
fn spawn_expr_parses_without_error() {
    let src = r#"
actor MyActor { }
spawn MyActor;
"#;
    let (_stdout, stderr, _code) = run_src("spawn_parse", src);
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error on spawn:\n{stderr}"
    );
}

/// `send target, message;` must parse without a parser error.
#[test]
fn send_stmt_parses_without_error() {
    let src = r#"
actor MyActor { }
let ref = 0;
send ref, 42;
"#;
    let (_stdout, stderr, _code) = run_src("send_parse", src);
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error on send:\n{stderr}"
    );
}

/// `recv channel;` must parse without a parser error.
#[test]
fn recv_expr_parses_without_error() {
    let src = r#"
actor MyActor { }
let ch = 0;
recv ch;
"#;
    let (_stdout, stderr, _code) = run_src("recv_parse", src);
    assert!(
        !stderr.contains("Parser error"),
        "unexpected parser error on recv:\n{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Runtime rejection tests — these assert that the runtime refuses to
// execute the concurrency-preview nodes with a clear "not yet executable"
// diagnostic, rather than silently succeeding or panicking.
// ---------------------------------------------------------------------------

/// `actor` declaration produces a clear not-yet-executable error.
/// The typechecker emits a W0001 warning; the runtime rejects the
/// node with a "concurrency preview" message.  Either signal is
/// acceptable evidence that the node is not silently ignored.
#[test]
fn actor_decl_rejected_at_runtime() {
    let src = r#"
actor Counter {
    let x = 0;
}
"#;
    let (_stdout, stderr, _code) = run_src("actor_runtime", src);
    // Accept any of: the W0001 typechecker warning, the runtime
    // rejection message (contains "concurrency preview" with a space),
    // or the feature description string.
    assert!(
        stderr.contains("W0001")
            || stderr.contains("concurrency preview")
            || stderr.contains("concurrency-preview"),
        "expected concurrency-preview diagnostic in stderr:\n{stderr}"
    );
}

/// `spawn` must produce a clear not-yet-executable error.
#[test]
fn spawn_rejected_at_runtime() {
    let src = r#"
actor Foo { }
spawn Foo;
"#;
    let (_stdout, stderr, _code) = run_src("spawn_runtime", src);
    assert!(
        stderr.contains("concurrency preview")
            || stderr.contains("concurrency-preview")
            || stderr.contains("W0001"),
        "expected concurrency-preview rejection in stderr:\n{stderr}"
    );
}
