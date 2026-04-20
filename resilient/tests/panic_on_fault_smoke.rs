//! RES-211: integration tests for the `--panic-on-fault` flag.
//!
//! A `live { ... }` block that always faults is driven through
//! the real `resilient` binary both with and without the flag.
//!
//! - With `--panic-on-fault`: the first fault aborts the process
//!   with exit code 1, and stderr carries the `[fault]` marker
//!   plus a pointer at `--no-panic-on-fault`.
//! - Without the flag: the block exhausts its retry budget and
//!   the driver prints a `Live block failed after N attempts`
//!   diagnostic — crucially lacking the `[fault]` marker from
//!   the new code path.

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_resilient")
}

/// Write a tiny program that always faults inside a `live` block
/// to a fresh temp file. Returns the path (caller is responsible
/// for cleanup if they care — OS tmpdir churn handles it).
fn write_fault_program() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_211_fault_{}_{}.rs",
        std::process::id(),
        n
    ));
    let src = "\
fn main(int _d) {\n\
    live {\n\
        assert(false, \"seeded fault\");\n\
    }\n\
}\n\
main(0);\n";
    let mut f = std::fs::File::create(&path).expect("create tmp .rs");
    f.write_all(src.as_bytes()).expect("write tmp .rs");
    path
}

#[test]
fn panic_on_fault_aborts_immediately() {
    let prog = write_fault_program();
    let output = Command::new(bin())
        .arg("--panic-on-fault")
        .arg(&prog)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "--panic-on-fault must exit 1 on the first fault; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("[fault] --panic-on-fault"),
        "stderr should mention the --panic-on-fault abort marker; got:\n{stderr}"
    );
    // The retry-exhaustion diagnostic from the default path must
    // NOT appear — that's the whole point of the flag.
    assert!(
        !stderr.contains("Live block failed after"),
        "panic-on-fault should skip the retry loop; stderr={stderr}"
    );
}

#[test]
fn default_retries_then_exhausts() {
    let prog = write_fault_program();
    let output = Command::new(bin())
        .arg(&prog)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Without the flag, the run still fails — but via exhaustion,
    // not the immediate-abort path. The diagnostic should come
    // from `eval_live_block`'s "Live block failed after ..." branch.
    assert_eq!(
        output.status.code(),
        Some(1),
        "faulting program should still exit 1 by default; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("Live block failed after"),
        "default path should surface the retry-exhaustion diagnostic; got:\n{stderr}"
    );
    // And crucially, the --panic-on-fault marker must be absent.
    assert!(
        !stderr.contains("[fault] --panic-on-fault"),
        "default path should not emit the panic-on-fault marker; stderr={stderr}"
    );
}

#[test]
fn no_panic_on_fault_overrides_panic_on_fault() {
    // RES-211: `--no-panic-on-fault` after `--panic-on-fault`
    // restores the default retry behaviour. Useful for wrapper
    // scripts that want to opt out of a globally-set flag.
    let prog = write_fault_program();
    let output = Command::new(bin())
        .arg("--panic-on-fault")
        .arg("--no-panic-on-fault")
        .arg(&prog)
        .output()
        .expect("spawn resilient");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Live block failed after"),
        "`--no-panic-on-fault` should restore retry exhaustion; got:\n{stderr}"
    );
    assert!(
        !stderr.contains("[fault] --panic-on-fault"),
        "override should suppress the panic-on-fault marker; got:\n{stderr}"
    );
}

#[test]
fn help_lists_panic_on_fault() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("spawn resilient");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--panic-on-fault"),
        "--help should mention --panic-on-fault; got:\n{stdout}"
    );
}
