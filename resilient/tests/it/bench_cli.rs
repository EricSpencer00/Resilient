//! RES-2613: smoke tests for `rz bench <file>`.

use std::fs;
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
        "res_bench_cli_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn bench_runs_example_and_prints_ns_per_op_table() {
    let output = Command::new(bin())
        .args(["bench", "examples/bench_simple.rz"])
        .output()
        .expect("spawn resilient bench");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Benchmark") && stdout.contains("ns/op"),
        "expected benchmark table header; stdout={stdout}"
    );
    assert!(
        stdout.contains("empty block")
            && stdout.contains("integer arithmetic")
            && stdout.contains("fibonacci(10)"),
        "expected benchmark names in output; stdout={stdout}"
    );
    assert!(
        stdout.contains("mean")
            && stdout.contains("median")
            && stdout.contains("stddev")
            && stdout.contains("min")
            && stdout.contains("max"),
        "expected summary statistics columns; stdout={stdout}"
    );
}

#[test]
fn bench_supports_git_baseline_comparison() {
    let repo = tmp_dir("baseline");
    let src_path = repo.join("bench_sample.rz");
    fs::write(
        &src_path,
        "\
fn fib(int n) -> int {\n\
    if n <= 1 {\n\
        return n;\n\
    }\n\
    return fib(n - 1) + fib(n - 2);\n\
}\n\
\n\
bench \"fib\" {\n\
    fib(10);\n\
}\n",
    )
    .expect("write baseline source");

    let output = Command::new("git")
        .args(["init", "-q"])
        .arg(&repo)
        .output()
        .expect("git init");
    assert!(
        output.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let status = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "config",
            "user.email",
            "bench@test.local",
        ])
        .status()
        .expect("git config email");
    assert!(status.success(), "git config email failed");

    let status = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "config",
            "user.name",
            "Bench Test",
        ])
        .status()
        .expect("git config name");
    assert!(status.success(), "git config name failed");

    let status = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "add", "bench_sample.rz"])
        .status()
        .expect("git add");
    assert!(status.success(), "git add failed");

    let output = Command::new("git")
        .args([
            "-C",
            repo.to_str().unwrap(),
            "commit",
            "-q",
            "-m",
            "baseline",
        ])
        .output()
        .expect("git commit");
    assert!(
        output.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::write(
        &src_path,
        "\
fn fib(int n) -> int {\n\
    if n <= 1 {\n\
        return n;\n\
    }\n\
    return fib(n - 1) + fib(n - 2);\n\
}\n\
\n\
bench \"fib\" {\n\
    fib(12);\n\
}\n",
    )
    .expect("write current source");

    let output = Command::new(bin())
        .arg("bench")
        .arg(&src_path)
        .args(["--baseline", "HEAD"])
        .output()
        .expect("spawn resilient bench --baseline");

    assert_eq!(
        output.status.code(),
        Some(0),
        "expected exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("baseline") || stdout.contains("delta"),
        "expected baseline comparison output; stdout={stdout}"
    );

    let _ = fs::remove_dir_all(&repo);
}
