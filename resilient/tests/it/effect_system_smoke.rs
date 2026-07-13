//! RES-389: integration smoke tests for the `pure` / `io` effect
//! system.
//!
//! Two scenarios are covered end-to-end through the `resilient
//! check` CLI:
//!
//! - A program where a `pure fn` only calls other `pure fn`s and
//!   pure builtins — `check` must exit 0.
//! - A program where a `pure fn` calls an `io fn` — `check` must
//!   exit 1 and the diagnostic must read
//!   `cannot call io function ... from pure context`.
//!
//! These tests deliberately use the `resilient check` binary so
//! they also validate the diagnostic surface end users see, not
//! just the typechecker's internal return type.
//!
//! The happy-path golden example
//! `examples/pure_effect_demo.rz` covers runtime behaviour via
//! the standard golden harness (`examples_golden.rs`); this file
//! is focused on the compile-time rejection path.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_effect_smoke_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

/// Positive path: a pure caller → pure callee. Everything is
/// side-effect-free, so `resilient check` exits 0.
#[test]
fn pure_calling_pure_typechecks() {
    let dir = tmp_dir("pure_ok");
    let src = dir.join("ok.rz");
    std::fs::write(
        &src,
        "pure fn square(int x) { return x * x; }\n\
         pure fn fourth(int x) { return square(square(x)); }\n\
         fn main(int _d) { return 0; }\n\
         main(0);\n",
    )
    .unwrap();

    let out = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0 for pure→pure program; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Negative path: a pure fn tries to call an io fn. The check
/// pass must reject with the canonical diagnostic so IDEs and
/// users can pattern-match on it.
#[test]
fn pure_calling_io_is_rejected() {
    let dir = tmp_dir("pure_violates");
    let src = dir.join("bad.rz");
    std::fs::write(
        &src,
        "io   fn noisy(int x) { println(x); return x; }\n\
         pure fn caller(int x) { return noisy(x); }\n\
         fn main(int _d) { return 0; }\n\
         main(0);\n",
    )
    .unwrap();

    let out = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 for pure→io violation; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}{stdout}");
    assert!(
        combined.contains("cannot call io function `noisy` from pure context"),
        "expected effect-violation diagnostic; stdout={stdout} stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Bare `fn` (no annotation) defaults to `io`, so calling it
/// from a `pure` caller must also be rejected. This locks in
/// the backward-compat default promised by the ticket.
#[test]
fn pure_calling_unannotated_is_rejected() {
    let dir = tmp_dir("pure_to_unann");
    let src = dir.join("bad.rz");
    std::fs::write(
        &src,
        "fn      helper(int x) { return x + 1; }\n\
         pure fn caller(int x) { return helper(x); }\n\
         fn main(int _d) { return 0; }\n\
         main(0);\n",
    )
    .unwrap();

    let out = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1 — unannotated fns default to io; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// `pure` and `io` are soft keywords: a file that never uses
/// them in the function-decl position keeps full backward
/// compatibility. This check is what lets existing tests /
/// user programs survive unchanged.
#[test]
fn plain_fn_still_typechecks() {
    let dir = tmp_dir("plain");
    let src = dir.join("ok.rz");
    std::fs::write(
        &src,
        "fn helper(int x) { return x + 1; }\n\
         fn main(int _d) { let y = helper(3); return y; }\n\
         main(0);\n",
    )
    .unwrap();

    let out = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        out.status.code(),
        Some(0),
        "expected exit 0 for plain program; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
