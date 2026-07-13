//! RES-2831: the typechecker must reject `[]` indexing of a
//! non-indexable target at compile time, instead of deferring to a
//! runtime "Cannot index ..." fault. Arrays, strings, and maps stay
//! indexable; everything else is a static type error.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn check_src(src: &str) -> (String, Option<i32>) {
    static CTR: AtomicUsize = AtomicUsize::new(0);
    let n = CTR.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf =
        std::env::temp_dir().join(format!("res_2831_{}_{}.rz", std::process::id(), n));
    std::fs::write(&path, src).expect("write tmp");
    let out = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn rz check");
    let _ = std::fs::remove_file(&path);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (combined, out.status.code())
}

#[test]
fn index_int_target_is_rejected() {
    let (out, code) = check_src("let x = 5;\nlet z = x[0];\nprintln(z);\n");
    assert_eq!(
        code,
        Some(1),
        "indexing an int must fail typecheck; got:\n{out}"
    );
    assert!(
        out.contains("cannot index"),
        "expected a 'cannot index' diagnostic; got:\n{out}"
    );
}

#[test]
fn index_struct_target_is_rejected() {
    let (out, code) =
        check_src("struct P { int a }\nlet p = new P { a: 1 };\nlet z = p[0];\nprintln(z);\n");
    assert_eq!(
        code,
        Some(1),
        "indexing a struct must fail typecheck; got:\n{out}"
    );
    assert!(
        out.contains("cannot index"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn index_assignment_to_int_target_is_rejected() {
    let (out, code) = check_src("let x = 5;\nx[0] = 9;\nprintln(x);\n");
    assert_eq!(
        code,
        Some(1),
        "index-assignment to an int must fail typecheck; got:\n{out}"
    );
    assert!(
        out.contains("cannot index"),
        "expected diagnostic; got:\n{out}"
    );
}

#[test]
fn array_index_still_accepted() {
    let (out, code) = check_src("let xs = [1, 2, 3];\nlet z = xs[0];\nprintln(z);\n");
    assert_eq!(code, Some(0), "array indexing must still pass; got:\n{out}");
}

#[test]
fn string_index_still_accepted() {
    let (out, code) = check_src("let s = \"hi\";\nlet z = s[0];\nprintln(z);\n");
    assert_eq!(
        code,
        Some(0),
        "string indexing must still pass; got:\n{out}"
    );
}
