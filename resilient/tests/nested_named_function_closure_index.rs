//! RES-4486: named nested functions must not collide with later closures.

use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn run_source(source: &str, vm: bool) -> std::process::Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "res_nested_fn_closure_index_{}_{}.rz",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, source).expect("write nested function regression source");

    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    if vm {
        command.arg("--vm");
    }
    let output = command
        .arg(&path)
        .output()
        .expect("run nested function source");
    let _ = fs::remove_file(path);
    output
}

#[test]
fn named_nested_function_and_later_closure_keep_distinct_vm_indices() {
    let source = r#"
        fn outer() -> int {
            fn named() -> int { return 41; }
            let closure = fn() -> int { return 7; };
            let closure_value = closure();
            let named_value = named();
            return closure_value * 10 + named_value;
        }
        print(outer());
    "#;

    for vm in [false, true] {
        let output = run_source(source, vm);
        assert!(
            output.status.success(),
            "{} execution failed: {}",
            if vm { "VM" } else { "tree-walker" },
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "111",
            "{} returned the wrong result",
            if vm { "VM" } else { "tree-walker" }
        );
    }
}
