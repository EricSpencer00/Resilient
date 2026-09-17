use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_param_destructuring_enforcement_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    fs::create_dir_all(&path).expect("create temporary directory");
    path
}

fn run_check(path: &Path) -> (String, String, Option<i32>) {
    let output = Command::new(bin())
        .arg("check")
        .arg(path)
        .output()
        .expect("run rz check");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn malformed_tuple_parameter_is_rejected_by_the_compiler() {
    let dir = tmp_dir("malformed");
    let source = dir.join("broken.rz");
    fs::write(
        &source,
        "fn broken((int, int, int) _a_b) -> int { return 1; }\nfn main() { return 0; }\nmain();\n",
    )
    .expect("write source");

    let (_stdout, stderr, code) = run_check(&source);
    assert_eq!(
        code,
        Some(1),
        "malformed tuple parameter must fail; stderr={stderr}"
    );
    assert!(
        stderr.contains("param_destructuring:")
            && stderr.contains("destructures 2 local names")
            && stderr.contains("3 element(s)"),
        "expected tuple-arity diagnostic; stderr={stderr}"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn valid_tuple_parameter_reaches_the_compiler_without_note_noise() {
    let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("examples/param_destructuring_validation.rz");
    let (stdout, stderr, code) = run_check(&example);
    assert_eq!(
        code,
        Some(0),
        "valid tuple parameter must pass; stderr={stderr}"
    );
    assert!(
        !stderr.contains("tuple destructure") && !stderr.contains("param_destructuring:"),
        "valid tuple parameter must not emit implementation notes; stderr={stderr}"
    );
    assert!(
        stdout.contains("param_destructuring_validation.rz: ok"),
        "check should report success; stdout={stdout}"
    );
}
