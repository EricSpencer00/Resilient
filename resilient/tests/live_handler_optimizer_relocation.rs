//! RES-4558: optimizer passes must relocate live retry entry PCs.
//!
//! A fold before a `live` block changes the bytecode address of the block
//! body. The tree-walker is the semantic oracle: the VM must retry from the
//! same body start and preserve the observed result.

use std::process::Command;

fn run_source(tag: &str, vm: bool) -> std::process::Output {
    let mut path = std::env::temp_dir();
    path.push(format!("res_4558_{tag}_{}.rz", std::process::id()));
    let source = r#"
        static let fails_left = 1;

        fn main(int _d) -> int {
            if len("x") > 0 {
                println("pre");
            }
            let seen = 0;
            live retries(2) {
                seen = live_retries();
                if fails_left > 0 {
                    fails_left = 0;
                    assert(false);
                }
            }
            return seen;
        }

        println(main(0));
    "#;
    std::fs::write(&path, source).expect("write RES-4558 source");

    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    command
        .arg(&path)
        .arg("--seed=1")
        .env("RESILIENT_CONST_FOLD", "1");
    if vm {
        command.arg("--vm");
    }
    let output = command.output().expect("run RES-4558 source");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn vm_retries_from_relocated_body_start() {
    let interpreter = run_source("interpreter", false);
    assert!(
        interpreter.status.success(),
        "interpreter oracle failed: stdout={} stderr={}",
        String::from_utf8_lossy(&interpreter.stdout),
        String::from_utf8_lossy(&interpreter.stderr)
    );
    assert!(
        String::from_utf8_lossy(&interpreter.stdout)
            .lines()
            .any(|line| line.trim() == "1"),
        "interpreter should observe retry count 1; stdout={}",
        String::from_utf8_lossy(&interpreter.stdout)
    );

    let vm = run_source("vm", true);
    assert!(
        vm.status.success(),
        "VM diverged from interpreter: stdout={} stderr={}",
        String::from_utf8_lossy(&vm.stdout),
        String::from_utf8_lossy(&vm.stderr)
    );
    assert!(
        String::from_utf8_lossy(&vm.stdout)
            .lines()
            .any(|line| line.trim() == "1"),
        "VM should observe retry count 1; stdout={}",
        String::from_utf8_lossy(&vm.stdout)
    );
}
