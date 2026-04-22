//! RES-210: live-block semantics integration tests.
//!
//! These tests pin down the contract documented in
//! `docs/live-block-semantics.md`. They drive the compiled
//! `resilient` binary against short `.rs` programs written to a
//! temp dir and assert on combined stdout / stderr plus exit code.
//!
//! Why binary-level, not unit-level: the tree walker's live-block
//! loop touches enough subsystems (env snapshotting, the thread-
//! local retry stack, `live_retries()`, invariant re-check order,
//! nested error-chaining, `println`) that exercising it through the
//! driver is the cheapest way to catch regressions across layers
//! without re-specifying the AST.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Write `src` to a temp `.rs` file, run the resilient driver
/// against it, and return `(stdout, stderr, exit_code)`. The file
/// is removed on success; a deliberate leak on failure would let a
/// reviewer inspect the offending source by hand.
fn run_src(tag: &str, src: &str) -> (String, String, Option<i32>) {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("res_210_{tag}_{}.rs", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).expect("create temp .rs");
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

// --- Spec test 1: normal exit on first attempt ---

#[test]
fn case_01_body_succeeds_first_attempt_exits_zero() {
    // The body raises no error. `live_retries()` reads 0 on the
    // only attempt, the block returns its body's value, and the
    // driver exits 0.
    let src = "\
        static let seen = [];\n\
        fn main(int _d) {\n\
            live {\n\
                seen = push(seen, live_retries());\n\
                let r = 1 + 1;\n\
            }\n\
            println(seen);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, stderr, code) = run_src("case01", src);
    assert_eq!(
        code,
        Some(0),
        "success-on-first-attempt must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("[0]"),
        "live_retries() should read 0 on only attempt; stdout={stdout}"
    );
}

// --- Spec test 2: single retry, then success ---

#[test]
fn case_02_body_retries_once_then_succeeds() {
    // First attempt fails via assert; the second attempt
    // succeeds. `live_retries()` observes 0 then 1.
    let src = "\
        static let fails_left = 1;\n\
        static let seen = [];\n\
        fn main(int _d) {\n\
            live {\n\
                seen = push(seen, live_retries());\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced once\");\n\
                }\n\
                let ok = 42;\n\
            }\n\
            println(seen);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, _stderr, code) = run_src("case02", src);
    assert_eq!(code, Some(0), "should succeed on retry; stdout={stdout}");
    assert!(
        stdout.contains("[0, 1]"),
        "live_retries() should read 0,1; stdout={stdout}"
    );
}

// --- Spec test 3: max-retry exhaustion propagates ---

#[test]
fn case_03_max_retry_exceeded_propagates_error() {
    // The body always fails, exhausting the default MAX_RETRIES=3
    // cap. The driver exits non-zero and the stderr carries the
    // "Live block failed after 3 attempts" prefix.
    let src = "\
        fn main(int _d) {\n\
            live {\n\
                assert(false, \"always\");\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (_stdout, stderr, code) = run_src("case03", src);
    assert_ne!(code, Some(0), "exhaustion must propagate as non-zero exit");
    assert!(
        stderr.contains("Live block failed after 3 attempts"),
        "expected exhaustion prefix; stderr={stderr}"
    );
    assert!(
        stderr.contains("retry depth: 1"),
        "expected retry-depth footer; stderr={stderr}"
    );
}

// --- Spec test 4: invariant violation triggers retry ---

#[test]
fn case_04_invariant_failure_triggers_retry() {
    // The body itself never raises, but an invariant clause
    // fails on the first two attempts and holds on the third.
    // The retry arm treats an invariant violation the same as a
    // body-level error: bump retry_count, sleep (zero here),
    // restore env, re-run.
    let src = "\
        static let flips = 0;\n\
        static let seen = [];\n\
        fn main(int _d) {\n\
            live invariant flips >= 3 {\n\
                seen = push(seen, live_retries());\n\
                flips = flips + 1;\n\
            }\n\
            println(seen);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, stderr, code) = run_src("case04", src);
    // The block should succeed on the third attempt (counter 0,1,2).
    assert_eq!(
        code,
        Some(0),
        "invariant that eventually holds should succeed; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("[0, 1, 2]"),
        "live_retries() across invariant-driven retries; stdout={stdout}"
    );
    // The interpreter logs each invariant failure on stderr.
    assert!(
        stderr.contains("Invariant violation"),
        "expected invariant-violation diagnostic; stderr={stderr}"
    );
}

// --- Spec test 5: nested live blocks — inner exhaustion propagates ---

#[test]
fn case_05_nested_inner_exhaustion_propagates_to_outer() {
    // Inner always fails -> inner exhausts after 3 inner attempts
    // -> outer treats that as ONE failure and retries inner 3 more
    // times, 3 outer attempts total. The final error message
    // chains both retry-depth notes.
    let src = "\
        static let inner_calls = 0;\n\
        fn always_fail() {\n\
            inner_calls = inner_calls + 1;\n\
            assert(false, \"inner\");\n\
            return 0;\n\
        }\n\
        fn main(int _d) {\n\
            live {\n\
                live {\n\
                    let r = always_fail();\n\
                }\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (_stdout, stderr, code) = run_src("case05", src);
    assert_ne!(code, Some(0), "nested exhaustion must propagate");
    // Both levels should appear in the chained error.
    assert!(
        stderr.contains("retry depth: 1"),
        "expected outer depth marker; stderr={stderr}"
    );
    assert!(
        stderr.contains("retry depth: 2"),
        "expected inner depth marker; stderr={stderr}"
    );
    assert!(
        stderr.contains("Live block failed after 3 attempts"),
        "expected exhaustion prefix on at least one level; stderr={stderr}"
    );
}

// --- Spec test 6: live block inside a function call ---

#[test]
fn case_06_live_block_inside_function_body() {
    // A `live { ... }` that sits inside a plain fn body must
    // behave identically to a top-level one: the enclosing
    // function observes the block's success / failure just like
    // any ordinary statement.
    let src = "\
        static let fails_left = 2;\n\
        fn flaky() {\n\
            live {\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"forced\");\n\
                }\n\
                let ok = 7;\n\
            }\n\
            return 99;\n\
        }\n\
        fn main(int _d) {\n\
            let r = flaky();\n\
            println(r);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, _stderr, code) = run_src("case06", src);
    assert_eq!(code, Some(0), "flaky() should eventually return");
    assert!(
        stdout.contains("99"),
        "fn-wrapped live block should let fn continue after success; stdout={stdout}"
    );
}

// --- Spec test 7: `live_retries()` returns the correct count ---

#[test]
fn case_07_live_retries_counts_up_across_attempts() {
    // The body records `live_retries()` into a static array on
    // each attempt. With two forced failures we expect
    // [0, 1, 2]. This is the positive complement of the
    // unit-level tests in main.rs — running through the real
    // driver catches regressions in argv / println integration.
    let src = "\
        static let fails_left = 2;\n\
        static let seen = [];\n\
        fn main(int _d) {\n\
            live {\n\
                seen = push(seen, live_retries());\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"retry me\");\n\
                }\n\
                let done = 1;\n\
            }\n\
            println(seen);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, _stderr, code) = run_src("case07", src);
    assert_eq!(code, Some(0), "block should succeed on third attempt");
    assert!(
        stdout.contains("[0, 1, 2]"),
        "live_retries() must read 0,1,2 across attempts; stdout={stdout}"
    );
}

// --- Spec test 8: side-effect isolation contract ---

#[test]
fn case_08_static_let_survives_retry_regular_let_does_not() {
    // The contract (docs/live-block-semantics.md §7):
    //   * Regular `let` bindings INSIDE the body are dropped on
    //     retry — they're re-declared each attempt.
    //   * `static let` bindings live in `self.statics` and
    //     persist across retries and across the whole program —
    //     they are NOT automatically rolled back.
    //
    // We exercise both sides. `persistent` increments on every
    // body attempt; after 3 attempts (2 failures + 1 success) it
    // MUST read 3, proving no roll-back of `static let`.
    let src = "\
        static let persistent = 0;\n\
        static let fails_left = 2;\n\
        fn main(int _d) {\n\
            live {\n\
                persistent = persistent + 1;\n\
                if fails_left > 0 {\n\
                    fails_left = fails_left - 1;\n\
                    assert(false, \"roll me back (but static let won't)\");\n\
                }\n\
                let done = 1;\n\
            }\n\
            println(persistent);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, _stderr, code) = run_src("case08", src);
    assert_eq!(code, Some(0));
    // If static let WERE rolled back, `persistent` would reset to
    // 0 at the top of each retry and the final value would be 1.
    // The contract says we MUST see all three increments.
    assert!(
        stdout.contains("3"),
        "static let must persist across retries (NO automatic rollback); stdout={stdout}"
    );
    assert!(
        !stdout.contains("1\n"),
        "static let must NOT reset to 1 — that would be rollback; stdout={stdout}"
    );
}

// --- Spec test 9: custom retry budget via `retries N` ---

#[test]
fn case_09_custom_retry_budget_retries_5() {
    // RES-359: `live retries 5 { ... }` should allow up to 5 retries
    // before exhaustion. The body always fails, so we expect exhaustion
    // after 5 attempts with the error message showing "failed after 5 attempts".
    let src = "\
        static let attempts = 0;\n\
        fn main(int _d) {\n\
            live retries 5 {\n\
                attempts = attempts + 1;\n\
                assert(false, \"always fail\");\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (_stdout, stderr, code) = run_src("case09", src);
    assert_ne!(code, Some(0), "exhaustion must propagate as non-zero exit");
    assert!(
        stderr.contains("Live block failed after 5 attempts"),
        "expected exhaustion with 5 attempts; stderr={stderr}"
    );
    assert!(
        stderr.contains("retry depth: 1"),
        "expected retry-depth footer; stderr={stderr}"
    );
}

// --- Spec test 10: custom retry budget 1 exhausts immediately ---

#[test]
fn case_10_custom_retry_budget_retries_1() {
    // RES-359: `live retries 1 { ... }` should allow only 1 retry
    // before exhaustion (2 total attempts: initial + 1 retry).
    let src = "\
        static let attempts = [];\n\
        fn main(int _d) {\n\
            live retries 1 {\n\
                attempts = push(attempts, live_retries());\n\
                assert(false, \"fail\");\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (_stdout, stderr, code) = run_src("case10", src);
    assert_ne!(code, Some(0), "exhaustion must propagate as non-zero exit");
    assert!(
        stderr.contains("Live block failed after 1 attempts"),
        "expected exhaustion with 1 attempts; stderr={stderr}"
    );
}

// --- Spec test 11: retries 0 means no retries (fail on first error) ---

#[test]
fn case_11_custom_retry_budget_retries_0() {
    // RES-359: `live retries 0 { ... }` should fail immediately on
    // first error without any retries.
    let src = "\
        static let attempts = 0;\n\
        fn main(int _d) {\n\
            live retries 0 {\n\
                attempts = attempts + 1;\n\
                assert(false, \"fail immediately\");\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (_stdout, stderr, code) = run_src("case11", src);
    assert_ne!(code, Some(0), "exhaustion must propagate as non-zero exit");
    assert!(
        stderr.contains("Live block failed after 0 attempts"),
        "expected exhaustion with 0 attempts (no retries); stderr={stderr}"
    );
}

// --- Spec test 12: retries can be combined with other clauses ---

#[test]
fn case_12_retries_with_backoff_and_invariant() {
    // RES-359: `retries N` should work alongside `backoff(...)` and
    // `invariant` clauses. The order should not matter.
    let src = "\
        static let flips = 0;\n\
        static let seen = [];\n\
        fn main(int _d) {\n\
            live retries 2 backoff(base_ms=1, factor=2, max_ms=10) invariant flips >= 2 {\n\
                seen = push(seen, live_retries());\n\
                flips = flips + 1;\n\
            }\n\
            println(seen);\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, _stderr, code) = run_src("case12", src);
    assert_eq!(
        code,
        Some(0),
        "block should succeed after 2 attempts; stdout={stdout}"
    );
    assert!(
        stdout.contains("[0, 1]"),
        "live_retries() should show attempts 0,1; stdout={stdout}"
    );
}
