use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_typecheck(example: &str) -> (String, String, Option<i32>) {
    let output = Command::new(bin())
        .arg("--typecheck")
        .arg(format!("examples/{example}"))
        .output()
        .expect("spawn resilient --typecheck");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn actor_value_payload_example_typechecks() {
    let (_stdout, stderr, code) = run_typecheck("actor_value_payload_ok.rz");
    assert_eq!(
        code,
        Some(0),
        "by-value actor payload example should typecheck; stderr={stderr}"
    );
}

#[test]
fn actor_reference_state_example_is_rejected() {
    let (_stdout, stderr, code) = run_typecheck("actor_ref_state_bad.rz");
    assert_ne!(
        code,
        Some(0),
        "reference-typed actor state should be rejected"
    );
    assert!(
        stderr.contains("ownership-by-value") && stderr.contains("state field"),
        "expected actor-isolation diagnostic; stderr={stderr}"
    );
}

#[test]
fn actor_reference_payload_example_is_rejected() {
    let (_stdout, stderr, code) = run_typecheck("actor_ref_payload_bad.rz");
    assert_ne!(
        code,
        Some(0),
        "reference-typed actor payload should be rejected"
    );
    assert!(
        stderr.contains("ownership-by-value") && stderr.contains("parameter `msg`"),
        "expected actor-isolation diagnostic; stderr={stderr}"
    );
}
