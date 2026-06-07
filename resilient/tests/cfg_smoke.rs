//! RES-2988: end-to-end smoke coverage for cfg/feature/target flows.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_rz(args: &[&str]) -> (String, String, Option<i32>) {
    let output = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn resilient binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn feature_flag_selects_verbose_branch() {
    let (stdout, _stderr, code) = run_rz(&["examples/cfg_feature.rz"]);
    assert_eq!(code, Some(0), "default cfg_feature run should succeed");
    assert!(
        stdout.contains("quiet mode"),
        "default cfg_feature output should be quiet mode; got:\n{stdout}"
    );

    let (stdout, _stderr, code) = run_rz(&["--feature", "verbose", "examples/cfg_feature.rz"]);
    assert_eq!(code, Some(0), "--feature verbose run should succeed");
    assert!(
        stdout.contains("verbose mode"),
        "verbose feature should activate verbose branch; got:\n{stdout}"
    );
}

#[test]
fn target_flag_selects_thumb_branch() {
    let (stdout, _stderr, code) = run_rz(&["examples/cfg_target.rz"]);
    assert_eq!(code, Some(0), "default cfg_target run should succeed");
    assert!(
        stdout.contains("42"),
        "default target should keep host branch; got:\n{stdout}"
    );

    let (stdout, _stderr, code) = run_rz(&["--target", "thumbv7em", "examples/cfg_target.rz"]);
    assert_eq!(code, Some(0), "--target thumbv7em run should succeed");
    assert!(
        stdout.contains("0"),
        "thumbv7em target should activate the thumb branch; got:\n{stdout}"
    );
}

#[test]
fn cfg_key_value_selects_custom_branch() {
    let (stdout, _stderr, code) = run_rz(&["examples/cfg_kv_demo.rz"]);
    assert_eq!(code, Some(0), "default cfg_kv_demo run should succeed");
    assert!(
        stdout.contains("default mode"),
        "default key/value cfg branch should be selected; got:\n{stdout}"
    );

    let (stdout, _stderr, code) = run_rz(&["--cfg", "mode=demo", "examples/cfg_kv_demo.rz"]);
    assert_eq!(code, Some(0), "--cfg mode=demo run should succeed");
    assert!(
        stdout.contains("demo mode"),
        "mode=demo should activate the custom cfg branch; got:\n{stdout}"
    );
}
