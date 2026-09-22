//! RES-4584: strict termination must cover named nested functions.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn run_source(source: &str) -> std::process::Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "res_4584_nested_termination_{}_{}.rz",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, source).expect("write nested termination source");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--strict-termination"])
        .arg(&path)
        .output()
        .expect("run nested termination source");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn unannotated_nested_recursion_is_rejected() {
    let output = run_source(
        "fn outer(int n) -> int {\n\
            fn inner(int x) -> int {\n\
                if x <= 0 { return 0; }\n\
                return inner(x - 1) + 1;\n\
            }\n\
            return inner(n);\n\
        }\n\
        outer(3);\n",
    );
    assert!(
        !output.status.success(),
        "unannotated nested recursion was accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("function `inner` is directly recursive"),
        "{stderr}"
    );
    assert!(
        stderr.contains("@decreases") && stderr.contains("@may_diverge"),
        "{stderr}"
    );
}

#[test]
fn annotated_nested_recursion_is_accepted() {
    let output = run_source(
        "fn outer(int n) -> int {\n\
            // @decreases x\n\
            fn inner(int x) -> int {\n\
                if x <= 0 { return 0; }\n\
                return inner(x - 1) + 1;\n\
            }\n\
            return inner(n);\n\
        }\n\
        outer(3);\n",
    );
    assert!(
        output.status.success(),
        "annotated nested recursion was rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
