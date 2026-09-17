//! RES-4260: end-to-end validation for `#[peripheral]` declarations.
//!
//! These cases invoke the strict CLI so they exercise attribute collection,
//! typechecking, and the hardware state-machine extension pass together.

use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_source(name: &str, source: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "res_hw_state_machine_{}_{}.rz",
        std::process::id(),
        name
    ));
    std::fs::write(&path, source).expect("write peripheral source");
    let output = Command::new(bin())
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

const VALID_PROGRAM: &str = r#"
#[peripheral(states = "Reset Configured", transitions = "Reset:configure->Configured")]
struct Usb { int handle }

fn main() { println("ok") }
main();
"#;

#[test]
fn valid_peripheral_declaration_still_runs() {
    let output = run_source("valid", VALID_PROGRAM);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid peripheral failed:\nstdout={stdout}\nstderr={stderr}"
    );
    assert!(
        stdout.contains("ok"),
        "valid peripheral did not run: {stdout}"
    );
}

#[test]
fn malformed_peripheral_declarations_are_rejected_with_source_diagnostics() {
    let cases = [
        (
            "missing-states",
            r#"#[peripheral(transitions = "Reset:init->Configured")]"#,
            "requires a `states",
        ),
        (
            "invalid-transition",
            r#"#[peripheral(states = "Reset Configured", transitions = "garbage")]"#,
            "malformed transition `garbage`",
        ),
        (
            "undefined-target",
            r#"#[peripheral(states = "Reset Configured", transitions = "Reset:init->Missing")]"#,
            "targets undeclared state `Missing`",
        ),
        (
            "undefined-source",
            r#"#[peripheral(states = "Reset Configured", transitions = "Missing:init->Configured")]"#,
            "source `Missing` is not in `states`",
        ),
        (
            "empty-states",
            r#"#[peripheral(states = "", transitions = "Reset:init->Configured")]"#,
            "declares no states",
        ),
        (
            "duplicate-state",
            r#"#[peripheral(states = "Reset Configured Reset")]"#,
            "duplicate state `Reset`",
        ),
        (
            "empty-method",
            r#"#[peripheral(states = "Reset Configured", transitions = "Reset:->Configured")]"#,
            "empty method name",
        ),
        (
            "unknown-key",
            r#"#[peripheral(state = "Reset")]"#,
            "unknown #[peripheral] argument `state`",
        ),
    ];

    for (name, attribute, expected) in cases {
        let source = format!(
            "{attribute}\nstruct Usb {{ int handle }}\nfn main() {{ println(\"ok\") }}\nmain();\n"
        );
        let output = run_source(name, &source);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{name} unexpectedly passed:\nstdout={}\nstderr={stderr}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            stderr.contains(expected),
            "{name} missing {expected:?}: {stderr}"
        );
        assert!(
            stderr.contains(":1:1:"),
            "{name} lost the attribute source location: {stderr}"
        );
        assert!(
            stderr.contains("Usb"),
            "{name} lost the item name: {stderr}"
        );
    }
}

#[test]
fn duplicate_peripheral_attributes_are_rejected() {
    let output = run_source(
        "duplicate",
        r#"
#[peripheral(states = "Reset Configured")]
#[peripheral(states = "Off On")]
struct Usb { int handle }
fn main() { println("ok") }
main();
"#,
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "duplicate peripheral unexpectedly passed:\n{stderr}"
    );
    assert!(stderr.contains("duplicate #[peripheral] registration for `Usb`"));
    assert!(
        stderr.contains("first registered at line 2"),
        "unexpected duplicate diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("duplicate at line 3"),
        "unexpected duplicate diagnostic: {stderr}"
    );
    assert!(
        stderr.contains(":3:1:"),
        "diagnostic lost duplicate location: {stderr}"
    );
}

#[test]
fn peripheral_without_transitions_is_valid() {
    let output = run_source(
        "no-transitions",
        r#"
#[peripheral(states = "Reset Configured")]
struct Usb { int handle }
fn main() { println("ok") }
main();
"#,
    );
    assert!(
        output.status.success(),
        "states-only peripheral failed:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
