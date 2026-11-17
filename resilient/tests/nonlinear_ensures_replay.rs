//! RES-5171: regressions found with rz 1.2.0.
//!
//! 1. An ensures clause with a nonlinear product must emit certificates
//!    that stock Z3 replays as `unsat`.
//! 2. A valid ensures over a branching body must not be refuted.
//! 3. A genuinely false overflow bound must still be refuted with a
//!    counterexample.
#![cfg(feature = "z3")]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("res_5171_{}_{}_{}", tag, std::process::id(), n));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn write_program(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("program.rz");
    let source = format!("{body}\nfn main() {{ println(f(1, 2)); }}\nmain();\n");
    std::fs::write(&path, source).expect("write source");
    path
}

fn z3_available() -> bool {
    Command::new("z3").arg("-version").output().is_ok()
}

const NARROW_BOUND: &str = "fn f(int den, int fac) -> int
    requires den > 0 && den < 16384 && fac >= 0 && fac <= 562949953421312
    ensures result == den * fac && result <= 9223372036854775807
{ return den * fac; }";

const WIDE_BOUND: &str = "fn f(int den, int fac) -> int
    requires den > 0 && den < 16384 && fac >= 0 && fac <= 1125899906842624
    ensures result == den * fac && result <= 9223372036854775807
{ return den * fac; }";

const BRANCH_LEMMA: &str = "fn f(int a, int b) -> int
    requires a >= 0 && b >= 0 && a != b
    ensures result == 1
{ if a / 2 != b / 2 || a % 2 != b % 2 { return 1; } return 0; }";

#[test]
fn nonlinear_ensures_certificates_replay_unsat_in_stock_z3() {
    let dir = tmp_dir("narrow");
    let src = write_program(&dir, NARROW_BOUND);
    let certs = dir.join("certs");
    let out = Command::new(bin())
        .arg("--typecheck")
        .arg("--emit-certificate")
        .arg(&certs)
        .arg(&src)
        .output()
        .expect("spawn rz");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let smt2: Vec<PathBuf> = std::fs::read_dir(&certs)
        .expect("certificate directory")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "smt2"))
        .collect();
    assert!(
        !smt2.is_empty(),
        "expected at least one certificate for the ensures clause"
    );
    if !z3_available() {
        eprintln!("z3 binary not on PATH; skipping independent replay");
        return;
    }
    for path in smt2 {
        let replay = Command::new("z3")
            .arg("-smt2")
            .arg(&path)
            .output()
            .expect("spawn z3");
        let stdout = String::from_utf8_lossy(&replay.stdout);
        assert_eq!(
            stdout.trim(),
            "unsat",
            "certificate {} did not replay as unsat:\n{}\n--- certificate ---\n{}",
            path.display(),
            stdout,
            std::fs::read_to_string(&path).unwrap_or_default()
        );
    }
}

#[test]
fn false_overflow_bound_is_still_refuted() {
    let dir = tmp_dir("wide");
    let src = write_program(&dir, WIDE_BOUND);
    let out = Command::new(bin())
        .arg("--typecheck")
        .arg(&src)
        .output()
        .expect("spawn rz");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_ne!(
        out.status.code(),
        Some(0),
        "expected refutation; stderr={stderr}"
    );
    assert!(
        stderr.contains("violates"),
        "expected an ensures violation; stderr={stderr}"
    );
}

#[test]
fn valid_branching_ensures_is_not_refuted() {
    let dir = tmp_dir("branch");
    let src = write_program(&dir, BRANCH_LEMMA);
    let out = Command::new(bin())
        .arg("--typecheck")
        .arg(&src)
        .output()
        .expect("spawn rz");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("violates"),
        "a valid ensures over a branching body was refuted; stderr={stderr}"
    );
    assert_eq!(out.status.code(), Some(0), "stderr={stderr}");
}
