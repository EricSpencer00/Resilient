//! RES-4111: native JIT String comparisons must match the interpreter.

#[cfg(feature = "jit")]
#[test]
fn string_comparisons_execute_natively_and_match_expected_output() {
    use std::process::Command;

    let example = format!(
        "{}/tests/fixtures/jit_native_string_comparison.rz",
        env!("CARGO_MANIFEST_DIR")
    );
    let interpreter = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&example)
        .output()
        .expect("spawn rz interpreter");
    assert_eq!(interpreter.status.code(), Some(0));
    let interpreter_stdout = String::from_utf8_lossy(&interpreter.stdout);
    let expected = [
        "false",
        "true",
        "true",
        "true",
        "true",
        "true",
        "true",
        "true",
        "Program executed successfully",
    ]
    .join("\n")
        + "\n";
    assert_eq!(interpreter_stdout, expected);

    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--jit", "--verbose", &example])
        .output()
        .expect("spawn rz --jit");

    assert_eq!(
        output.status.code(),
        Some(0),
        "JIT string comparison example failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let jit_lines = stdout.lines().skip(2).collect::<Vec<_>>();
    assert_eq!(
        &jit_lines[..8],
        &expected.lines().take(8).collect::<Vec<_>>()[..]
    );
    assert_eq!(&jit_lines[8..], &["0", "Program executed successfully"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("fell back to the VM"),
        "string comparisons should execute through native JIT lowering: {stderr}"
    );
}
