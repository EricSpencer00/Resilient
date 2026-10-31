use std::process::Command;

#[test]
fn controlled_requirements_public_pipeline() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let result = Command::new("python3")
        .arg(root.join("qualification/requirements-pipeline/tests/test_cli.py"))
        .env("RZ_BIN", env!("CARGO_BIN_EXE_rz"))
        .output()
        .expect("run public requirements pipeline");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
