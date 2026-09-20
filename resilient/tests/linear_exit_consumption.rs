//! RES-4592: linear values must be discharged before a function exits.

use std::fs;
use std::process::Command;

fn run_typecheck(source: &str, tag: &str) -> std::process::Output {
    let path = std::env::temp_dir().join(format!(
        "res_4592_{}_{}_{}.rz",
        std::process::id(),
        tag,
        source.len()
    ));
    fs::write(&path, source).expect("write RES-4592 fixture");
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("--typecheck")
        .arg(&path)
        .output()
        .expect("run rz --typecheck");
    let _ = fs::remove_file(path);
    output
}

#[test]
fn unconsumed_linear_local_is_rejected_at_function_scope_exit() {
    let output = run_typecheck(
        "struct FileHandle { int fd }\n\
         fn forget_local() {\n\
             let fh: linear FileHandle = new FileHandle { fd: 1 };\n\
             return 0;\n\
         }\n",
        "function-local",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr.contains("not consumed before scope `forget_local` exits"),
        "expected the function-exit diagnostic, got:\n{stderr}"
    );
}

#[test]
fn unconsumed_linear_local_in_nested_scope_is_rejected() {
    let output = run_typecheck(
        "struct FileHandle { int fd }\n\
         fn forget_nested() {\n\
             if true {\n\
                 let fh: linear FileHandle = new FileHandle { fd: 1 };\n\
             }\n\
             return 0;\n\
         }\n",
        "nested-local",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_ne!(output.status.code(), Some(0));
    assert!(
        stderr.contains("linear value `fh: linear FileHandle`"),
        "expected the local binding in the diagnostic, got:\n{stderr}"
    );
}

#[test]
fn return_and_definite_branch_consumption_remain_valid() {
    let output = run_typecheck(
        "struct FileHandle { int fd }\n\
         fn consume(linear FileHandle fh) { return 0; }\n\
         fn forward(linear FileHandle fh) { return fh; }\n\
         fn conditional(linear FileHandle fh, int flag) {\n\
             if flag == 1 { consume(fh); } else { consume(fh); }\n\
             return 0;\n\
         }\n",
        "valid",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "ownership-return and definite branch consumption should typecheck; stderr={stderr}"
    );
}
