use std::process::Command;

#[test]
fn independent_requirements_correspondence() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let result = Command::new("python3")
        .arg(root.join("qualification/requirements-pipeline/tests/test_independent.py"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .output()
        .expect("run independent correspondence checks");
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
