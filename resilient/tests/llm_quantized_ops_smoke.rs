use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn llm_quantized_matmul_example_runs_cpu_fallback_kernel() {
    let output = Command::new(bin())
        .arg("examples/llm_quantized_matmul.rz")
        .output()
        .expect("failed to run llm quantized matmul example");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "example should exit cleanly; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("[42, -1, -22, -9]"),
        "expected flattened 2x2 q8 matmul output, got:\n{stdout}"
    );
}
