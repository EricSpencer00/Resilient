#![cfg(feature = "logos-lexer")]

use std::process::Command;

#[test]
fn logos_lexer_preserves_reserved_keywords_at_cli_boundary() {
    let path = std::env::temp_dir().join(format!("res_logos_keywords_{}.rz", std::process::id()));
    std::fs::write(&path, "assume extern const enum unsafe pub defer bench")
        .expect("write Logos keyword fixture");

    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args([
            "--dump-tokens",
            path.to_str().expect("fixture path is UTF-8"),
        ])
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(&path);

    assert!(
        output.status.success(),
        "token dump failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for keyword in [
        "Assume(\"assume\")",
        "Extern(\"extern\")",
        "Const(\"const\")",
        "Enum(\"enum\")",
        "Unsafe(\"unsafe\")",
        "Pub(\"pub\")",
        "Defer(\"defer\")",
        "Bench(\"bench\")",
    ] {
        assert!(
            stdout.contains(keyword),
            "Logos token dump omitted {keyword}: {stdout}"
        );
    }
}
