use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

#[test]
fn help_lists_mcp_http_hosting_flag() {
    let output = Command::new(bin())
        .arg("--help")
        .output()
        .expect("failed to run rz --help");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--mcp-http-port ADDR"), "got:\n{stdout}");
    assert!(stdout.contains("rz mcp --http-port 8080"), "got:\n{stdout}");
}
