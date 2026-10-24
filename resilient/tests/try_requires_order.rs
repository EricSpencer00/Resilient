//! Declared checked-failure injection must not bypass callee preconditions.
use sha2::{Digest, Sha256};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn run(source: &str, backend: &str) -> std::process::Output {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "try_requires_order_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).unwrap();
    let input = root.join("input.rz");
    std::fs::write(&input, source).unwrap();
    let mut args = vec!["--typecheck", "--no-cache"];
    if backend != "treewalker" {
        args.push("--vm");
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command
        .args(&args)
        .arg(&input)
        .env_remove("RESILIENT_OVERFLOW_MODE");
    if backend != "treewalker" {
        command.env("RESILIENT_DISPATCH", backend);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            std::fs::remove_dir_all(&root).unwrap();
            panic!("bounded CLI timeout: {backend}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    let record = serde_json::json!({
        "schema": "try-requires-order-v2", "backend": backend,
        "source": source, "source_sha256": format!("{:x}", Sha256::digest(source.as_bytes())),
        "argv": args, "overflow_mode": "unset",
        "compiler_sha256": format!("{:x}", Sha256::digest(std::fs::read(env!("CARGO_BIN_EXE_rz")).unwrap())),
        "exit": output.status.code(), "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr)
    });
    std::io::stdout()
        .lock()
        .write_all(format!("TRY_REQUIRES_ORDER_RECORD {record}\n").as_bytes())
        .unwrap();
    std::fs::remove_dir_all(root).unwrap();
    output
}

const INVALID: &str = "fn negative() -> int { let n = -1; return n; } fn risky(int x) -> int requires x > 0 fails Failure { return x; } try { println(risky(negative())); } catch Failure { println(\"FORBIDDEN_CATCH\"); }";
const VALID: &str = "fn positive() -> int { let n = 1; return n; } fn risky(int x) -> int requires x > 0 fails Failure { return x; } try { println(risky(positive())); } catch Failure { println(\"EXPECTED_CATCH\"); }";

fn invalid(backend: &str) {
    let output = run(INVALID, backend);
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(out.contains("Type check passed"), "{output:?}");
    assert!(!output.status.success(), "{output:?}");
    assert!(!out.contains("FORBIDDEN_CATCH"), "{output:?}");
    assert!(
        err.contains("requires") && err.contains("x > 0"),
        "{output:?}"
    );
}
fn valid(backend: &str) {
    let output = run(VALID, backend);
    let out = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(out.contains("Type check passed"), "{output:?}");
    assert!(
        out.lines().any(|line| line == "EXPECTED_CATCH"),
        "{output:?}"
    );
}
#[test]
fn treewalker_requires_precedes_injection() {
    invalid("treewalker");
}
#[test]
fn vm_match_requires_precedes_injection() {
    invalid("match");
}
#[test]
fn vm_direct_requires_precedes_injection() {
    invalid("direct");
}
#[test]
fn treewalker_valid_call_injects_declared_failure() {
    valid("treewalker");
}
#[test]
fn vm_match_valid_call_injects_declared_failure() {
    valid("match");
}
#[test]
fn vm_direct_valid_call_injects_declared_failure() {
    valid("direct");
}
