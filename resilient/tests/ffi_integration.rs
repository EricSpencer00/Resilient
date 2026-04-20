//! End-to-end FFI integration tests against the bundled C helper library.
//!
//! These tests require `--features ffi` and the presence of a system C
//! compiler (`cc`). `build.rs` handles the C compilation and injects the
//! resulting library path into `RESILIENT_FFI_TESTHELPER_PATH`, which we
//! splice into each Resilient source string below so the `extern "..."`
//! library descriptor points at the freshly-built `.so`/`.dylib`.
//!
//! Task 9 of FFI Phase 1: covers Int→Int, Bool return, contract
//! pre-condition failure, and missing-symbol error handling.

#![cfg(all(feature = "ffi", any(target_os = "linux", target_os = "macos")))]

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Path to the compiled test helper library (injected by build.rs).
fn helper_path() -> &'static str {
    env!("RESILIENT_FFI_TESTHELPER_PATH")
}

fn resilient_bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Monotonically-increasing suffix so parallel test runs don't collide
/// on the same temp-file name. `std::process::id()` would work too, but
/// four tests in one binary would share it — combine pid + counter for
/// safety.
fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Write a Resilient source string to a temp file and run it with the
/// `resilient` binary. Returns (stdout, stderr, exit_code).
fn run_resilient_src(src: &str) -> (String, String, i32) {
    let tmp = std::env::temp_dir().join(format!(
        "res_ffi_task9_{}_{}.rs",
        std::process::id(),
        next_seq()
    ));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp file");
        f.write_all(src.as_bytes()).expect("write tmp file");
    }
    let output = Command::new(resilient_bin())
        .arg(&tmp)
        .output()
        .expect("failed to spawn resilient binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_file(&tmp);
    (stdout, stderr, code)
}

#[test]
fn calls_int_int_int_function() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_add(a: Int, b: Int) -> Int; }};
fn main(int _d) {{
    println(rt_add(2, 40));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "42"),
        "expected a line with `42`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn calls_bool_function() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_is_even(n: Int) -> Bool; }};
fn main(int _d) {{
    println(rt_is_even(4));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "true"),
        "expected a line with `true`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn contract_precondition_failure_is_caught_before_ffi_call() {
    // Resilient contracts use paren-LESS syntax: `requires EXPR`.
    // The tree-walker binds extern-fn params positionally as `_0`, `_1`,
    // ..., not by source name — so the contract references `_0`
    // (the first arg) rather than `a`.
    let src = format!(
        r#"extern "{lib}" {{ fn rt_add(a: Int, b: Int) -> Int requires _0 >= 0; }};
fn main(int _d) {{
    println(rt_add(-1, 1));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, _code) = run_resilient_src(&src);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.to_lowercase().contains("contract violation"),
        "expected contract violation, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn missing_symbol_is_clean_error_not_panic() {
    let src = format!(
        r#"extern "{lib}" {{ fn definitely_not_a_symbol() -> Int; }};
fn main(int _d) {{
    println(definitely_not_a_symbol());
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    let combined = format!("{stdout}{stderr}");
    // Should fail gracefully, not panic — exit code != 0 and message mentions
    // either `symbol` (from FfiError::SymbolNotFound) or `FFI`.
    assert!(code != 0, "expected non-zero exit, got stdout={stdout} stderr={stderr}");
    assert!(
        combined.contains("symbol") || combined.contains("FFI"),
        "expected FFI/symbol error, got stdout={stdout} stderr={stderr}"
    );
}
