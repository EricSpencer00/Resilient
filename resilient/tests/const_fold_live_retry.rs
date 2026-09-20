use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_const_fold_live_retry_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write live retry fixture");
    path
}

fn run(source: &str, vm: bool) -> Output {
    let path = scratch_file(source);
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    if vm {
        command.arg("--vm");
    }
    let output = command
        .env("RESILIENT_CONST_FOLD", "1")
        .arg(&path)
        .output()
        .expect("spawn rz live retry fixture");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn vm_remaps_live_retry_body_after_constant_fold() {
    let source = r#"
static let fails_left = 1;

fn main(int _d) -> int {
    let folded = 1 + 2;
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

    let interpreter = run(source, false);
    let vm = run(source, true);

    assert!(
        interpreter.status.success(),
        "tree-walk execution failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&interpreter.stdout),
        String::from_utf8_lossy(&interpreter.stderr)
    );
    assert!(
        vm.status.success(),
        "VM execution failed: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&vm.stdout),
        String::from_utf8_lossy(&vm.stderr)
    );
    assert_eq!(
        interpreter.stdout,
        vm.stdout,
        "tree-walk and VM output diverged after constant folding:\ninterpreter={:?}\nvm={:?}",
        String::from_utf8_lossy(&interpreter.stdout),
        String::from_utf8_lossy(&vm.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&vm.stdout),
        "1\nProgram executed successfully\n"
    );
}
