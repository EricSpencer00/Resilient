//! RES-4246: `extern fn` call sites are name-resolved and type-checked.
//!
//! Before this, `Node::Extern` validated the declared signature but
//! never bound the declared name, so every call site failed name
//! resolution with a non-fatal `Undefined variable` diagnostic — and
//! arity / argument types went unchecked until the FFI trampoline saw
//! them at runtime.
//!
//! These tests use `rz check`, which resolves no dynamic symbols, so
//! they run in the default (no `--features ffi`) build and need no
//! compiled helper library. The end-to-end tests that do call into C
//! live in `ffi_integration.rs`.

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn resilient_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Run `rz check` over a source string. Returns (stdout, stderr, exit code).
fn check_src(src: &str) -> (String, String, i32) {
    let tmp = std::env::temp_dir().join(format!(
        "res_extern_callsite_{}_{}.rz",
        std::process::id(),
        next_seq()
    ));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp file");
        f.write_all(src.as_bytes()).expect("write tmp file");
    }
    let output = Command::new(resilient_bin())
        .arg("check")
        .arg(&tmp)
        .output()
        .expect("failed to spawn rz");
    let _ = std::fs::remove_file(&tmp);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn correct_extern_call_produces_no_diagnostic() {
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_add(a: Int, b: Int) -> Int; };
fn main(int _d) { println(ext_add(1, 2)); }
main(0);"#,
    );
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    let all = format!("{stdout}{stderr}");
    assert!(
        !all.contains("Undefined variable"),
        "extern name must resolve at the call site, got: {all}"
    );
    assert!(
        !all.contains("Type error") && !all.contains("error:"),
        "a well-typed extern call must be diagnostic-free, got: {all}"
    );
}

#[test]
fn extern_call_resolves_before_its_declaration() {
    // The call textually precedes the `extern` block, so only the
    // hoisting pre-pass can resolve it.
    let (stdout, stderr, code) = check_src(
        r#"fn main(int _d) { println(ext_add(1, 2)); }
extern "libtesthelper" { fn ext_add(a: Int, b: Int) -> Int; };
main(0);"#,
    );
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        !format!("{stdout}{stderr}").contains("Undefined variable"),
        "forward reference to an extern must resolve: stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn wrong_arity_extern_call_is_rejected_at_compile_time() {
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_add(a: Int, b: Int) -> Int; };
fn main(int _d) { println(ext_add(1)); }
main(0);"#,
    );
    assert_ne!(code, 0, "must not check clean: stdout={stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("Expected 2 arguments, got 1"),
        "stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn wrong_argument_type_extern_call_is_rejected_at_compile_time() {
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_add(a: Int, b: Int) -> Int; };
fn main(int _d) { println(ext_add(1, "two")); }
main(0);"#,
    );
    assert_ne!(code, 0, "must not check clean: stdout={stdout}");
    assert!(
        format!("{stdout}{stderr}").contains("argument 2"),
        "expected an argument-2 type mismatch, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn extern_return_type_flows_into_the_caller() {
    // `-> Int` binding a `string` let is the proof that the declared
    // return type reaches the call site rather than erasing to `Any`.
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_add(a: Int, b: Int) -> Int; };
fn main(int _d) { string s = ext_add(1, 2); println(s); }
main(0);"#,
    );
    assert_ne!(code, 0, "must not check clean: stdout={stdout} {stderr}");
}

#[test]
fn cstr_parameter_accepts_a_string_and_rejects_an_int() {
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_len(s: CStr) -> Int; };
fn main(int _d) { println(ext_len("abcd")); }
main(0);"#,
    );
    assert_eq!(
        code, 0,
        "CStr takes a String: stdout={stdout} stderr={stderr}"
    );

    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_len(s: CStr) -> Int; };
fn main(int _d) { println(ext_len(7)); }
main(0);"#,
    );
    assert_ne!(code, 0, "must not check clean: stdout={stdout} {stderr}");
}

#[test]
fn array_parameter_accepts_an_array_and_rejects_a_scalar() {
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_sum(xs: Array<Int>, n: Int) -> Int; };
fn main(int _d) { println(ext_sum([1, 2, 3], 3)); }
main(0);"#,
    );
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");

    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_sum(xs: Array<Int>, n: Int) -> Int; };
fn main(int _d) { println(ext_sum(1, 3)); }
main(0);"#,
    );
    assert_ne!(code, 0, "must not check clean: stdout={stdout} {stderr}");
}

#[test]
fn variadic_extern_accepts_any_trailing_argument_count() {
    // A C variadic has no fixed arity; binding one as a fixed-arity
    // function would make every real `printf`-shaped call a compile
    // error. It resolves, and the arity check stays out of the way.
    let (stdout, stderr, code) = check_src(
        r#"extern "libtesthelper" { fn ext_printf(fmt: CStr, ...) -> Int32; };
fn main(int _d) { println(ext_printf("%d %d", 1, 2)); }
main(0);"#,
    );
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        !format!("{stdout}{stderr}").contains("Undefined variable"),
        "variadic extern name must resolve: stdout={stdout} stderr={stderr}"
    );
}
