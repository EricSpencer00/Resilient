//! Regression coverage for complete `#[power]` executable-path accounting.

use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn check_source(source: &str) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_power_contracts_regression_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write power-contract fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["check"])
        .arg(&path)
        .output()
        .expect("spawn rz check");
    std::fs::remove_file(path).expect("remove power-contract fixture");
    output
}

#[test]
fn power_budget_counts_try_body_and_catch_handler() {
    let output = check_source(
        r#"
fn read_sensor(int x) fails Timeout { return x; }
fn radio_send(int x) { return x; }

#[power(uj = "50")]
fn transmit(int x) {
    try {
        radio_send(x);
        read_sensor(x);
    } catch Timeout {
        radio_send(x);
    }
}

transmit(1);
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "try/catch power overrun was accepted: {stderr}"
    );
    assert!(
        stderr.contains("energy budget exceeded"),
        "expected power-budget diagnostic: {stderr}"
    );
}

#[test]
fn power_budget_counts_match_arm_calls() {
    let output = check_source(
        r#"
fn radio_send(int x) { return x; }

#[power(uj = "50")]
fn transmit(int x) {
    match x {
        0 => radio_send(x),
        _ => 0,
    };
}

transmit(1);
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "match-arm power overrun was accepted: {stderr}"
    );
    assert!(
        stderr.contains("energy budget exceeded"),
        "expected power-budget diagnostic: {stderr}"
    );
}
