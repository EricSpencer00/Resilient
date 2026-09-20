use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_const_vm_traversal_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write const traversal fixture");
    path
}

fn run(source: &str, vm: bool) -> Output {
    let path = scratch_file(source);
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    if vm {
        command.arg("--vm");
    }
    let output = command
        .arg(&path)
        .output()
        .expect("spawn rz const traversal fixture");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn vm_preserves_consts_across_expression_bearing_nodes() {
    let source = r#"
const LIMIT = 2;

fn summarize() -> int {
    assert(LIMIT == 2, "limit={LIMIT}");
    assume(LIMIT > 0);

    let values = [10, 20, 30];
    let window = values[0..LIMIT];
    let pair = (LIMIT, window[0]);
    let (limit, first) = pair;

    println("limit={LIMIT}; len={len(window)}; first={first}");
    return limit + first;
}

println(summarize());
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
        "tree-walk and VM output diverged:\ninterpreter={:?}\nvm={:?}",
        String::from_utf8_lossy(&interpreter.stdout),
        String::from_utf8_lossy(&vm.stdout)
    );
    assert_eq!(
        String::from_utf8_lossy(&vm.stdout),
        "limit=2; len=2; first=10\n12\nProgram executed successfully\n"
    );
}
