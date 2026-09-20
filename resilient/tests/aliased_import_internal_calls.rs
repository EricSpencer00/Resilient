//! RES-4656: aliased file imports preserve calls between imported declarations.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn temp_project() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_aliased_import_calls_{}_{}",
        std::process::id(),
        n
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("create aliased-import test directory");
    fs::write(
        path.join("lib.rz"),
        "pub fn helper() -> int { return 7; }\n\
         pub fn public() -> int { return helper(); }\n",
    )
    .expect("write imported module");
    fs::write(
        path.join("main.rz"),
        "use \"lib.rz\" as lib;\n\
         fn main() { println(lib::public()); }\n\
         main();\n",
    )
    .expect("write importing module");
    path
}

fn run(backend: Option<&str>, project: &PathBuf) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    if let Some(backend) = backend {
        command.arg(backend);
    }
    command
        .arg(project.join("main.rz"))
        .current_dir(project)
        .output()
        .expect("run aliased-import program")
}

fn assert_success(output: &std::process::Output, backend: &str) {
    assert_eq!(
        output.status.code(),
        Some(0),
        "{backend} aliased import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "7\n",
        "{backend} returned the wrong result"
    );
}

#[test]
fn tree_walker_preserves_internal_calls_in_aliased_imports() {
    let project = temp_project();
    let output = run(None, &project);
    assert_success(&output, "tree walker");
    let _ = fs::remove_dir_all(project);
}

#[test]
fn bytecode_vm_preserves_internal_calls_in_aliased_imports() {
    let project = temp_project();
    let output = run(Some("--vm"), &project);
    assert_success(&output, "bytecode VM");
    let _ = fs::remove_dir_all(project);
}
