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
    env!("CARGO_BIN_EXE_rz")
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
    assert!(
        code != 0,
        "expected non-zero exit, got stdout={stdout} stderr={stderr}"
    );
    assert!(
        combined.contains("symbol") || combined.contains("FFI"),
        "expected FFI/symbol error, got stdout={stdout} stderr={stderr}"
    );
}

// ============================================================
// RES-317: C struct bridging — small structs by value.
// ============================================================

#[test]
fn struct_bridging_int_to_struct_factory() {
    // Resilient declares `OneInt` as `@repr(C)` and calls a C factory
    // that returns a struct by value. The trampoline marshals the i64
    // returned in the INTEGER-class register back into a Resilient
    // `Value::Struct` and reads `.v` to print.
    let src = format!(
        r#"@repr(C) struct OneInt {{ Int v }}
extern "{lib}" {{ fn rt_make_one_int(v: Int) -> OneInt; }};
fn main(int _d) {{
    let s = rt_make_one_int(42);
    println(s.v);
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "42"),
        "expected `42`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn struct_bridging_struct_round_trip() {
    let src = format!(
        r#"@repr(C) struct OneInt {{ Int v }}
extern "{lib}" {{ fn rt_double_one_int(s: OneInt) -> OneInt; }};
fn main(int _d) {{
    let inp = new OneInt {{ v: 21 }};
    let out = rt_double_one_int(inp);
    println(out.v);
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "42"),
        "expected `42`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn struct_bridging_struct_to_int_readback() {
    let src = format!(
        r#"@repr(C) struct OneInt {{ Int v }}
extern "{lib}" {{ fn rt_one_int_value(s: OneInt) -> Int; }};
fn main(int _d) {{
    let inp = new OneInt {{ v: 99 }};
    println(rt_one_int_value(inp));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "99"),
        "expected `99`, got stdout={stdout} stderr={stderr}"
    );
}

// ============================================================
// RES-4225: `Array<Int>` / `Array<Float>` extern parameters.
// ============================================================

/// Assert that a Resilient source runs cleanly and prints `want` on some
/// line of stdout. Every array test below has that shape.
fn assert_prints(src: &str, want: &str) {
    let (stdout, stderr, code) = run_resilient_src(src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == want),
        "expected `{want}`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn array_int_param_reaches_c_as_contiguous_buffer() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int; }};
fn main(int _d) {{
    println(rt_sum_i64([1, 2, 3, 4], 4));
}}
main(0);"#,
            lib = helper_path()
        ),
        "10",
    );
}

#[test]
fn array_float_param_reaches_c_as_contiguous_buffer() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_sum_f64(xs: Array<Float>, n: Int) -> Float; }};
fn main(int _d) {{
    println(rt_sum_f64([1.5, 2.5], 2));
}}
main(0);"#,
            lib = helper_path()
        ),
        "4",
    );
}

#[test]
fn array_elements_are_in_declaration_order_not_reversed() {
    // Summing is order-insensitive, so it cannot catch a reversed or
    // rotated buffer. Index into it instead.
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_nth_i64(xs: Array<Int>, i: Int) -> Int; }};
fn main(int _d) {{
    println(rt_nth_i64([10, 20, 30], 0));
    println(rt_nth_i64([10, 20, 30], 2));
}}
main(0);"#,
            lib = helper_path()
        ),
        "30",
    );
}

#[test]
fn two_array_params_get_two_distinct_buffers() {
    // A single shared scratch buffer would make this print 14 (xs dotted
    // with itself) or 56 (ys with itself) instead of 32.
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_dot_i64(xs: Array<Int>, ys: Array<Int>, n: Int) -> Int; }};
fn main(int _d) {{
    println(rt_dot_i64([1, 2, 3], [4, 5, 6], 3));
}}
main(0);"#,
            lib = helper_path()
        ),
        "32",
    );
}

#[test]
fn array_variable_binding_is_marshalled_the_same_as_a_literal() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int; }};
fn main(int _d) {{
    let xs = [5, 7, 9];
    println(rt_sum_i64(xs, 3));
}}
main(0);"#,
            lib = helper_path()
        ),
        "21",
    );
}

#[test]
fn empty_array_is_passed_without_dereferencing() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int; }};
fn main(int _d) {{
    println(rt_sum_i64([], 0));
}}
main(0);"#,
            lib = helper_path()
        ),
        "0",
    );
}

#[test]
fn arity_eight_all_integer_class_dispatches_through_the_word_path() {
    // 8 params, two of them buffers — the upper bound of the arity-only
    // INTEGER-class table. (1+2) + (3+4+5) + 10+20+30+40 = 115.
    assert_prints(
        &format!(
            r#"extern "{lib}" {{
    fn rt_sum_two_bufs_8(a: Array<Int>, na: Int, b: Array<Int>, nb: Int,
                         w0: Int, w1: Int, w2: Int, w3: Int) -> Int;
}};
fn main(int _d) {{
    println(rt_sum_two_bufs_8([1, 2], 2, [3, 4, 5], 3, 10, 20, 30, 40));
}}
main(0);"#,
            lib = helper_path()
        ),
        "115",
    );
}

#[test]
fn mixed_element_types_are_rejected_at_runtime_with_the_offending_index() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int; }};
fn main(int _d) {{
    println(rt_sum_i64([1, 2.0], 2));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_ne!(code, 0, "mixed-type array must not run: stdout={stdout}");
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("[1]"),
        "diagnostic should name the offending index: {all}"
    );
}

#[test]
fn array_of_unsupported_element_type_is_rejected_at_compile_time() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<String>, n: Int) -> Int; }};
fn main(int _d) {{ println(0); }}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_ne!(code, 0, "Array<String> must be rejected: stdout={stdout}");
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("contiguous C layout"),
        "diagnostic should explain why the element type is unsupported: {all}"
    );
}

#[test]
fn array_return_type_is_rejected() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_sum_i64(xs: Array<Int>, n: Int) -> Array<Int>; }};
fn main(int _d) {{ println(0); }}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_ne!(code, 0, "array return must be rejected: stdout={stdout}");
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("lifetime") || all.contains("unsupported"),
        "diagnostic should explain why a C-allocated buffer cannot be returned: {all}"
    );
}

// ============================================================
// RES-4227: differential-oracle extern contracts.
//
// An `ensures` on an extern is a runtime check, not a proof. These tests
// pin the property that makes such a check worth writing: it can fail.
// See docs/ffi-trust-boundary.md.
// ============================================================

#[test]
fn extern_ensures_may_call_another_extern_as_a_reference() {
    // The differential-oracle pattern only exists if a contract
    // expression can call across the FFI boundary. Pin that.
    let src = format!(
        r#"extern "{lib}" {{
    fn rt_isqrt_ref(n: Int) -> Int;
    fn rt_isqrt_fast(n: Int) -> Int requires _0 >= 0 ensures result == rt_isqrt_ref(_0);
}};
fn main(int _d) {{
    println(rt_isqrt_fast(144));
    println(rt_isqrt_fast(1000000));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "12"),
        "expected `12`, got stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.lines().any(|l| l.trim() == "1000"),
        "expected `1000`, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn differential_oracle_fires_when_the_fast_path_disagrees() {
    // The whole point. `rt_isqrt_broken` is correct up to n = 1000 and
    // off by one above it, so this asserts both that the clause passes
    // when it should and fails when it should — a check that only ever
    // passes proves nothing.
    let src = format!(
        r#"extern "{lib}" {{
    fn rt_isqrt_ref(n: Int) -> Int;
    fn rt_isqrt_broken(n: Int) -> Int requires _0 >= 0 ensures result == rt_isqrt_ref(_0);
}};
fn main(int _d) {{
    println(rt_isqrt_broken(144));
    println(rt_isqrt_broken(1000000));
    println("unreachable");
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    let all = format!("{stdout}{stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "12"),
        "call below the divergence threshold should succeed: {all}"
    );
    assert!(
        all.contains("Contract violation"),
        "diverging call must raise a contract violation: {all}"
    );
    assert!(
        all.contains("rt_isqrt_broken"),
        "diagnostic must name the offending extern: {all}"
    );
    assert!(
        !stdout.lines().any(|l| l.trim() == "unreachable"),
        "execution must stop at the violation: {all}"
    );
    assert_ne!(code, 0, "a contract violation must not exit 0: {all}");
}

#[test]
fn wrapping_an_extern_keeps_the_wrappers_contract_on_our_side() {
    // The mitigation the docs recommend: put the reasoning in a
    // Resilient function whose body the verifier can actually see, and
    // leave only the call itself opaque.
    let src = format!(
        r#"extern "{lib}" {{ fn rt_isqrt_fast(n: Int) -> Int; }};
fn safe_isqrt(Int n) -> Int
    requires n >= 0
    ensures  result >= 0
{{
    if n == 0 {{ return 0; }}
    return rt_isqrt_fast(n);
}}
fn main(int _d) {{
    println(safe_isqrt(0));
    println(safe_isqrt(144));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "12"),
        "expected `12`, got stdout={stdout} stderr={stderr}"
    );
}

// ============================================================
// RES-4226: `Int32` (C `int`) and `CStr` (`const char*`).
// ============================================================

#[test]
fn int32_return_preserves_a_negative_error_code() {
    // The regression that motivated `Int32`. A C function returning
    // `int` writes only eax / w0; reading the full 64-bit return
    // register yields 4294967293 instead of -3. Declaring the return as
    // `Int32` transmutes to `-> i32` so only the defined bits are read.
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_neg_errcode() -> Int32; }};
fn main(int _d) {{
    println(rt_neg_errcode());
}}
main(0);"#,
            lib = helper_path()
        ),
        "-3",
    );
}

#[test]
fn int32_round_trips_negative_and_boundary_values() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_i32_identity(v: Int32) -> Int32; }};
fn main(int _d) {{
    println(rt_i32_identity(-2147483648));
    println(rt_i32_identity(2147483647));
    println(rt_i32_identity(-1));
}}
main(0);"#,
            lib = helper_path()
        ),
        "-2147483648",
    );
}

#[test]
fn int32_argument_out_of_range_is_refused_not_truncated() {
    // Wrapping would hand C a different number than the caller wrote —
    // the same class of silent corruption Int32 exists to prevent.
    let src = format!(
        r#"extern "{lib}" {{ fn rt_i32_identity(v: Int32) -> Int32; }};
fn main(int _d) {{
    println(rt_i32_identity(2147483648));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_ne!(code, 0, "out-of-range Int32 must not run: stdout={stdout}");
    let all = format!("{stdout}{stderr}");
    assert!(
        all.contains("does not fit in Int32"),
        "diagnostic should name the range problem: {all}"
    );
}

#[test]
fn int32_params_and_return_compose() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_i32_add(a: Int32, b: Int32) -> Int32; }};
fn main(int _d) {{
    println(rt_i32_add(-10, 4));
}}
main(0);"#,
            lib = helper_path()
        ),
        "-6",
    );
}

#[test]
fn cstr_param_reaches_c_nul_terminated() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_cstr_len(s: CStr) -> Int; }};
fn main(int _d) {{
    println(rt_cstr_len("hello"));
    println(rt_cstr_len(""));
}}
main(0);"#,
            lib = helper_path()
        ),
        "5",
    );
}

#[test]
fn two_cstr_params_get_two_distinct_buffers() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_cstr_eq(a: CStr, b: CStr) -> Int32; }};
fn main(int _d) {{
    println(rt_cstr_eq("abc", "abc"));
    println(rt_cstr_eq("abc", "abd"));
}}
main(0);"#,
            lib = helper_path()
        ),
        "0",
    );
}

#[test]
fn cstr_with_interior_nul_is_refused_not_truncated() {
    let src = format!(
        r#"extern "{lib}" {{ fn rt_cstr_len(s: CStr) -> Int; }};
fn main(int _d) {{
    println(rt_cstr_len("a\0b"));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    let all = format!("{stdout}{stderr}");
    // Truncating at the NUL would silently print 1. Either the string
    // never reaches C (clean error) or the length is the full 3 — what
    // must not happen is a silent 1.
    if code == 0 {
        assert!(
            !stdout.lines().any(|l| l.trim() == "1"),
            "interior NUL must not silently truncate: {all}"
        );
    } else {
        assert!(
            all.contains("interior NUL"),
            "diagnostic should name the interior NUL: {all}"
        );
    }
}

#[test]
fn cstr_return_is_copied_into_a_resilient_string() {
    assert_prints(
        &format!(
            r#"extern "{lib}" {{ fn rt_version_string() -> CStr; }};
fn main(int _d) {{
    println(rt_version_string());
}}
main(0);"#,
            lib = helper_path()
        ),
        "testhelper 1.0.0",
    );
}

#[test]
fn cstr_and_arrays_and_handles_mix_in_one_signature() {
    // The point of the arity-only dispatch: CStr, Array<Int>, and Int
    // are all one INTEGER-class register, so a mixed signature needs no
    // new trampoline arm.
    assert_prints(
        &format!(
            r#"extern "{lib}" {{
    fn rt_cstr_len(s: CStr) -> Int;
    fn rt_sum_i64(xs: Array<Int>, n: Int) -> Int;
}};
fn main(int _d) {{
    println(rt_cstr_len("abcd") + rt_sum_i64([1, 2, 3], 3));
}}
main(0);"#,
            lib = helper_path()
        ),
        "10",
    );
}

// ---------------------------------------------------------------
// RES-4246: call sites resolve the declared extern name.
// ---------------------------------------------------------------

#[test]
fn successful_extern_call_emits_no_undefined_variable_diagnostic() {
    // The extern name was never bound into the type environment, so a
    // working call printed `Undefined variable 'rt_add'` to stderr
    // while still exiting 0 — noise on every FFI program, and proof
    // that the call site was never type-checked at all.
    let src = format!(
        r#"extern "{lib}" {{ fn rt_add(a: Int, b: Int) -> Int; }};
fn main(int _d) {{
    println(rt_add(1, 2));
}}
main(0);"#,
        lib = helper_path()
    );
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|l| l.trim() == "3"),
        "stdout={stdout} stderr={stderr}"
    );
    assert!(
        !stderr.contains("Undefined variable"),
        "extern call must not emit a name-resolution diagnostic: {stderr}"
    );
    assert!(
        !stderr.contains("Type error"),
        "extern call must not emit a type error: {stderr}"
    );
}
