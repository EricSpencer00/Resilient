//! Regression coverage for RES-4563: newtype constructors must be lowered
//! wherever expressions can occur, including reassignment and indexing.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_newtype_lowering_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write newtype lowering fixture");
    path
}

#[test]
fn assignment_and_nested_expression_constructors_execute() {
    let path = scratch_file(
        r#"
newtype Meters = Int;

fn main() {
    let mut distance = Meters(1);
    distance = Meters(2);
    let readings = [Meters(3), Meters(4)];
    println(distance.__value);
    println(readings[1].__value);
}

main();
"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(&path)
        .output()
        .expect("spawn rz for newtype lowering regression");
    let _ = std::fs::remove_file(path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "newtype constructors in assignment/nested expressions must run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "2\n4\nProgram executed successfully\n",
        "the reassigned and indexed newtype values must retain their payloads"
    );
}
