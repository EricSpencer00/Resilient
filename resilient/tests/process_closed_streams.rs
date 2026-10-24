//! A successful process may close its output streams before it exits.
#![cfg(unix)]

use std::process::Command;

#[test]
fn exec_shell_accepts_closed_streams_before_successful_exit() {
    let root = std::env::temp_dir().join(format!("res_process_closed_{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("closed.rz");
    std::fs::write(
        &source,
        "let result = exec_shell(\"exec 1>&- 2>&-; sleep 0.2\"); println(is_ok(result));",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&source)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "true");
}
