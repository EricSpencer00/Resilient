use std::fs;
use std::process::{Command, Output};

fn check_source(source: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "resilient-no-alloc-{}-{}.rz",
        std::process::id(),
        source.len()
    ));
    fs::write(&path, source).expect("write no_alloc fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["check", path.to_str().expect("temporary path is UTF-8")])
        .output()
        .expect("run rz check");
    fs::remove_file(path).expect("remove no_alloc fixture");
    output
}

#[test]
fn no_alloc_rejects_call_to_uncertified_function() {
    let output = check_source(
        "fn make_array(int x) -> int { let a = [1, 2, 3]; return x; }\n\
         #[no_alloc]\n\
         fn wrapper(int x) -> int { return make_array(x); }\n",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "unexpected success: {stderr}");
    assert!(
        stderr.contains("uncertified function `make_array`"),
        "missing transitive no_alloc diagnostic: {stderr}"
    );
}

#[test]
fn no_alloc_accepts_certified_call_chain() {
    let output = check_source(
        "#[no_alloc]\n\
         fn add_one(int x) -> int { return x + 1; }\n\
         #[no_alloc]\n\
         fn wrapper(int x) -> int { return add_one(x); }\n",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "certified chain rejected: {stderr}"
    );
}
