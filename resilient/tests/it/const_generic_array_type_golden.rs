//! RES-4109 (A-E2 follow-up): golden coverage for the const-generic
//! `array<T, N>` spelling in `src/const_generic_len.rs`. Mirrors
//! `const_generic_len_golden.rs`, which covers the `[T; N]` bracket
//! form — both spellings share the same underlying `fixed_len` parser
//! and provable-site checks, so these tests confirm the angle-bracket
//! form gets identical treatment.

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
        "res_const_generic_array_type_golden_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn normalize_output(path: &Path, output: std::process::Output) -> String {
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

fn run_check(tag: &str, src: &str) -> String {
    let dir = tmp_dir(tag);
    let src_path = dir.join("main.rz");
    std::fs::write(&src_path, src).expect("write test source");
    let output = Command::new(bin())
        .arg("check")
        .arg(&src_path)
        .output()
        .expect("spawn rz check");
    let normalized = normalize_output(&src_path, output);
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

/// REJECT: `let xs: array<int, 3> = [1, 2];` — literal length 2
/// against a declared length 3.
#[test]
fn const_generic_array_type_let_mismatch_rejected_matches_golden() {
    let src = "fn main() { let xs: array<int, 3> = [1, 2]; println(xs[0]); }\nmain();\n";
    assert_eq!(
        run_check("let_err", src),
        read_expected("const_generic_array_type_let_err.txt")
    );
}

/// REJECT: a 2-element literal passed where the parameter is declared
/// `array<int, 3>`.
#[test]
fn const_generic_array_type_call_arg_mismatch_rejected_matches_golden() {
    let src = "fn sum3(int base, array<int, 3> v) -> int { return base + v[0] + v[1] + v[2]; }\n\
fn main() { println(sum3(1, [1, 2])); }\nmain();\n";
    assert_eq!(
        run_check("arg_err", src),
        read_expected("const_generic_array_type_arg_err.txt")
    );
}

/// REJECT: `return [\"x\"];` from a fn declared `-> array<string, 2>`.
#[test]
fn const_generic_array_type_return_mismatch_rejected_matches_golden() {
    let src = "fn axes() -> array<string, 2> { return [\"x\"]; }\nfn main() { println(axes()[0]); }\nmain();\n";
    assert_eq!(
        run_check("ret_err", src),
        read_expected("const_generic_array_type_ret_err.txt")
    );
}
