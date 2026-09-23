//! RES-4702: std::iter must fail closed when its signed cursor overflows.

use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn run_source(source: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "res_iter_range_signed_boundary_{}_{}.rz",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).expect("write range fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&path)
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(path);
    output
}

fn run_err(source: &str) -> String {
    let result = run_source(source);
    assert!(
        !result.status.success(),
        "expected range failure, got: {:?}",
        result.stdout
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    )
}

#[test]
fn positive_cursor_overflow_is_typed_error() {
    let errors = run_err(
        r#"
        use std::iter;
        fn main() {
            iter_range(9223372036854775806, 9223372036854775807, 2);
        }
        main();
        "#,
    );
    assert!(
        errors.contains("iter::range: step overflow at signed boundary"),
        "unexpected error: {errors}"
    );
}

#[test]
fn negative_cursor_overflow_is_typed_error() {
    let errors = run_err(
        r#"
        use std::iter;
        fn main() {
            iter_range(-9223372036854775807, (-9223372036854775807 - 1), -2);
        }
        main();
        "#,
    );
    assert!(
        errors.contains("iter::range: step overflow at signed boundary"),
        "unexpected error: {errors}"
    );
}

#[test]
fn ordinary_ranges_preserve_results() {
    let source = r#"
        use std::iter;
        fn main() {
            let ascending = iter_range(2, 6, 2);
            let descending = iter_range(6, 1, -2);
            println(len(ascending));
            println(ascending[1]);
            println(len(descending));
            println(descending[1]);
        }
        main();
        "#;
    let result = run_source(source);
    assert!(
        result.status.success(),
        "ordinary range failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);
    assert!(stdout.contains("2\n4\n3\n4"), "unexpected output: {stdout}");
}
