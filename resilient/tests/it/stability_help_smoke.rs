//! RES-3133: pin user-facing stability vocabulary in CLI help.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_file(tag: &str, body: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_3133_stability_{}_{}_{}.rz",
        tag,
        std::process::id(),
        n
    ));
    std::fs::write(&path, body).expect("write scratch file");
    path
}

#[test]
fn help_prints_canonical_stability_vocabulary() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn rz --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "STATUS:",
        "stable             Supported for scripts and CI on the default build",
        "backend-limited    Stable when the named backend/build feature is present;",
        "experimental       User-facing, but policy/output may still evolve",
        "--jit                    Route through the Cranelift JIT",
        "(backend-limited; requires --features jit)",
        "--lsp                    Run the LSP server on stdio",
        "(backend-limited; requires --features lsp)",
        "--dump-ast-json          Print the parsed AST as JSON and exit",
        "(experimental tooling surface)",
        "repl                 Start interactive REPL (alias for bare `rz`)",
    ] {
        assert!(
            stdout.contains(expected),
            "help output missing {expected:?}; got:\n{stdout}"
        );
    }
}

#[cfg(not(feature = "jit"))]
#[test]
fn jit_feature_gate_uses_backend_limited_language() {
    let path = tmp_file("jit_gate", "fn main() { return 1; }\nmain();\n");
    let output = Command::new(bin())
        .arg("--jit")
        .arg(&path)
        .output()
        .expect("spawn rz --jit");
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        output.status.code(),
        Some(1),
        "--jit without the jit feature should fail; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Backend-limited: --jit requires the `jit` feature")
            && stderr.contains("cargo build --features jit"),
        "--jit feature gate should use backend-limited wording; got:\n{stderr}"
    );
}

#[cfg(not(feature = "lsp"))]
#[test]
fn lsp_feature_gate_uses_backend_limited_language() {
    let output = Command::new(bin())
        .arg("--lsp")
        .output()
        .expect("spawn rz --lsp");

    assert_eq!(
        output.status.code(),
        Some(1),
        "--lsp without the lsp feature should fail; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Backend-limited: --lsp requires the `lsp` feature")
            && stderr.contains("cargo build --features lsp"),
        "--lsp feature gate should use backend-limited wording; got:\n{stderr}"
    );
}
