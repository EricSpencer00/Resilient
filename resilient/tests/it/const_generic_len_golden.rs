//! RES-4078 (A-E2): golden coverage for const-generic fixed-array
//! length checking (`src/const_generic_len.rs`). Provable mismatches
//! — a direct array literal against a `[T; N]` annotation — are
//! rejected at typecheck time; anything whose length is not
//! syntactically known stays accepted (zero false positives).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_const_generic_len_golden_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn normalize_output(path: &Path, output: std::process::Output, include_streams: bool) -> String {
    if !include_streams {
        return format!("exit={}\n", output.status.code().unwrap_or(-1));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let path_str = path.to_string_lossy();
    let combined = format!(
        "exit={}\n{}{}",
        output.status.code().unwrap_or(-1),
        stdout,
        stderr
    );
    combined.replace(path_str.as_ref(), "<tmp>.rz")
}

fn run_check(tag: &str, quiet: bool, src: &str) -> String {
    let dir = tmp_dir(tag);
    let src_path = dir.join("main.rz");
    std::fs::write(&src_path, src).expect("write test source");
    let mut cmd = Command::new(bin());
    cmd.arg("check");
    if quiet {
        cmd.arg("-q");
    }
    let output = cmd.arg(&src_path).output().expect("spawn rz check");
    let normalized = normalize_output(&src_path, output, !quiet);
    let _ = std::fs::remove_dir_all(&dir);
    normalized
}

fn read_expected(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(name);
    std::fs::read_to_string(path).expect("read golden")
}

/// ACCEPT: exact-length literals in every checked position, plus a
/// non-literal initializer whose length is unknowable (must stay
/// permissive).
#[test]
fn const_generic_len_exact_and_unknown_accepted_matches_golden() {
    let src = "fn sum3(int base, [int; 3] v) -> int { return base + v[0] + v[1] + v[2]; }\n\
fn axes() -> [string; 2] { return [\"x\", \"y\"]; }\n\
fn tail() { return [7, 8]; }\n\
fn main() {\n\
    let xs: [int; 3] = [10, 20, 30];\n\
    println(sum3(1, xs));\n\
    println(sum3(0, [1, 2, 3]));\n\
    let ys: [int; 3] = tail();\n\
    println(axes()[0]);\n\
}\n\
main();\n";
    assert_eq!(
        run_check("ok", true, src),
        read_expected("const_generic_len_ok.txt")
    );
}

/// REJECT: `let xs: [int; 3] = [1, 2];` — literal length 2 against a
/// declared length 3.
#[test]
fn const_generic_len_let_mismatch_rejected_matches_golden() {
    let src = "fn main() { let xs: [int; 3] = [1, 2]; println(xs[0]); }\nmain();\n";
    assert_eq!(
        run_check("let_err", false, src),
        read_expected("const_generic_len_let_err.txt")
    );
}

/// REJECT: a 2-element literal passed where the parameter is declared
/// `[int; 3]` — previously only an out-of-bounds error at runtime.
#[test]
fn const_generic_len_call_arg_mismatch_rejected_matches_golden() {
    let src = "fn sum3(int base, [int; 3] v) -> int { return base + v[0] + v[1] + v[2]; }\n\
fn main() { println(sum3(1, [1, 2])); }\nmain();\n";
    assert_eq!(
        run_check("arg_err", false, src),
        read_expected("const_generic_len_arg_err.txt")
    );
}

/// REJECT: `return [\"x\"];` from a fn declared `-> [string; 2]`.
#[test]
fn const_generic_len_return_mismatch_rejected_matches_golden() {
    let src = "fn axes() -> [string; 2] { return [\"x\"]; }\nfn main() { println(axes()[0]); }\nmain();\n";
    assert_eq!(
        run_check("ret_err", false, src),
        read_expected("const_generic_len_ret_err.txt")
    );
}
