use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_check(tag: &str, source: &str) -> (String, Option<i32>) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "res_row_poly_match_{tag}_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write row-poly match fixture");
    let output = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("spawn resilient check");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

const PREFIX: &str = r#"
#[row_poly(requires = "name:string level:int")]
fn log_event(any event) -> int { return event.level; }
struct Minimal { string name }
"#;

#[test]
fn rejects_invalid_row_poly_literal_in_match_arm() {
    let source = format!(
        "{PREFIX}\
         fn main() -> int {{\n\
             return match 0 {{\n\
                 0 => log_event(new Minimal {{ name: \"boot\" }}),\n\
                 _ => 0,\n\
             }};\n\
         }}\n\
         main();\n"
    );
    let (stderr, code) = run_check("arm", &source);
    assert_eq!(code, Some(1), "invalid arm should fail: {stderr}");
    assert!(
        stderr.contains("row-poly violation"),
        "missing row-poly diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("level"),
        "diagnostic should name the missing field: {stderr}"
    );
}

#[test]
fn rejects_invalid_row_poly_literal_in_match_scrutinee() {
    let source = format!(
        "{PREFIX}\
         fn main() -> int {{\n\
             return match log_event(new Minimal {{ name: \"boot\" }}) {{\n\
                 _ => 0,\n\
             }};\n\
         }}\n\
         main();\n"
    );
    let (stderr, code) = run_check("scrutinee", &source);
    assert_eq!(code, Some(1), "invalid scrutinee should fail: {stderr}");
    assert!(
        stderr.contains("row-poly violation"),
        "missing row-poly diagnostic: {stderr}"
    );
}

#[test]
fn rejects_invalid_row_poly_literal_in_match_guard() {
    let source = format!(
        "{PREFIX}\
         fn main() -> int {{\n\
             return match 0 {{\n\
                 n if log_event(new Minimal {{ name: \"boot\" }}) > 0 => 1,\n\
                 _ => 0,\n\
             }};\n\
         }}\n\
         main();\n"
    );
    let (stderr, code) = run_check("guard", &source);
    assert_eq!(code, Some(1), "invalid guard should fail: {stderr}");
    assert!(
        stderr.contains("row-poly violation"),
        "missing row-poly diagnostic: {stderr}"
    );
}

#[test]
fn accepts_valid_row_poly_literals_through_match_paths() {
    let source = r#"
#[row_poly(requires = "name:string level:int")]
fn log_event(any event) -> int { return event.level; }
struct Record { string name, int level }
fn main() -> int {
    return match log_event(new Record { name: "boot", level: 1 }) {
        n if log_event(new Record { name: "guard", level: n }) > 0 =>
            log_event(new Record { name: "arm", level: n }),
        _ => 0,
    };
}
main();
"#;
    let (stderr, code) = run_check("valid", source);
    assert_eq!(
        code,
        Some(0),
        "valid match paths should typecheck: {stderr}"
    );
}
