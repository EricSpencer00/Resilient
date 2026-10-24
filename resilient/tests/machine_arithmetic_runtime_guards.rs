//! Mathematical contract checks must not remove machine runtime enforcement.
#![cfg(feature = "z3")]
fn check(vm: bool) {
    let root = std::env::temp_dir().join(format!(
        "machine_runtime_guard_{}_{}",
        std::process::id(),
        vm
    ));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("input.rz"),
        "fn f(int x) -> int requires x + 1 > x { return x; } println(f(9223372036854775807));",
    )
    .unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_rz"));
    command
        .arg("--audit")
        .arg(root.join("input.rz"))
        .env_remove("RESILIENT_OVERFLOW_MODE");
    if vm {
        command.arg("--vm");
    }
    let output = command.output().unwrap();
    if let Some(destination) = std::env::var_os("MACHINE_ARITHMETIC_RECORDS") {
        let destination = std::path::PathBuf::from(destination).join(if vm {
            "vm_runtime_requires_control"
        } else {
            "treewalker_runtime_requires_not_elided"
        });
        std::fs::create_dir_all(&destination).unwrap();
        std::fs::copy(root.join("input.rz"), destination.join("input.rz")).unwrap();
        std::fs::write(destination.join("stdout.txt"), &output.stdout).unwrap();
        std::fs::write(destination.join("stderr.txt"), &output.stderr).unwrap();
        std::fs::write(
            destination.join("exit-status.txt"),
            output.status.to_string(),
        )
        .unwrap();
    }
    assert!(!output.status.success(), "{output:?}");
    let diagnostic = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        diagnostic.contains("Contract violation") && diagnostic.contains("requires"),
        "{diagnostic}"
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn treewalker_runtime_requires_not_elided() {
    check(false);
}
#[test]
fn vm_runtime_requires_control() {
    check(true);
}
