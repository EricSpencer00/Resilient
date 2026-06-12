use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_check(example: &str) -> (String, String, Option<i32>) {
    let output = Command::new(bin())
        .arg("check")
        .arg(format!(
            "examples/target_profiles_rejections/{example}/main.rz"
        ))
        .current_dir(manifest_dir())
        .output()
        .expect("spawn rz");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

fn assert_rejected(example: &str, needle: &str) {
    let (stdout, stderr, code) = run_check(example);
    assert_eq!(
        code,
        Some(1),
        "`rz check` should reject {example}; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.is_empty(),
        "typecheck rejection should not emit stdout; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("error[target-profiles]") && stderr.contains(needle),
        "expected target profile diagnostic containing `{needle}`; stderr={stderr}"
    );
}

#[test]
fn runtime_only_target_profile_failures_are_rejected_at_typecheck() {
    for (example, needle) in [
        ("features_not_array", "`features` must be a string array"),
        (
            "features_bad_entry",
            "`features` entries must be double-quoted strings",
        ),
        (
            "cfg_value_not_quoted",
            "cfg field `linker` must use a double-quoted string value",
        ),
        (
            "invalid_opt_level",
            "`opt_level` `fast`; expected one of: 0, 1, 2, 3, s",
        ),
        (
            "invalid_stack_size",
            "`stack_size` must be a positive integer, got `0`",
        ),
    ] {
        assert_rejected(example, needle);
    }
}
