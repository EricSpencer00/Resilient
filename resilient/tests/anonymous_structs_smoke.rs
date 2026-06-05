use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_anonymous_structs_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn anonymous_structs_typecheck_and_run() {
    let dir = tmp_dir("ok");
    let src = dir.join("anon_ok.rz");
    std::fs::write(
        &src,
        "fn point() -> { x: float, y: float } {\n\
         \x20\x20\x20\x20return { x: 1.0, y: 2.0 };\n\
         }\n\
         fn project_x({ x: float } p) -> float {\n\
         \x20\x20\x20\x20return p.x;\n\
         }\n\
         fn main(int _d) {\n\
         \x20\x20\x20\x20let p: { x: float } = point();\n\
         \x20\x20\x20\x20println(project_x({ x: 3.5, y: 9.0 }));\n\
         \x20\x20\x20\x20println(p.x);\n\
         \x20\x20\x20\x20return 0;\n\
         }\n\
         main(0);\n",
    )
    .unwrap();

    let check = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        check.status.code(),
        Some(0),
        "expected anonymous structs to typecheck; stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );

    let run = Command::new(bin())
        .arg(&src)
        .output()
        .expect("spawn resilient");
    assert_eq!(
        run.status.code(),
        Some(0),
        "expected anonymous structs to run; stdout={} stderr={}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("3.5") && stdout.contains("1"),
        "expected field-access output from anonymous structs; stdout={stdout}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn anonymous_struct_binding_rejects_missing_required_field() {
    let dir = tmp_dir("missing_field");
    let src = dir.join("anon_bad.rz");
    std::fs::write(
        &src,
        "fn main(int _d) {\n\
         \x20\x20\x20\x20let p: { x: int, y: int } = { x: 1 };\n\
         \x20\x20\x20\x20return 0;\n\
         }\n\
         main(0);\n",
    )
    .unwrap();

    let check = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        check.status.code(),
        Some(1),
        "expected missing field to fail typecheck; stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("y") || stderr.contains("value has type"),
        "expected a structural-typing diagnostic; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn row_poly_accepts_anonymous_struct_literal() {
    let dir = tmp_dir("row_poly");
    let src = dir.join("row_poly_anon.rz");
    std::fs::write(
        &src,
        "#[row_poly(requires = \"code:int\")]\n\
         fn emit(any event) -> int {\n\
         \x20\x20\x20\x20return event.code;\n\
         }\n\
         fn main(int _d) {\n\
         \x20\x20\x20\x20return emit({ code: 42, msg: \"ok\" });\n\
         }\n\
         main(0);\n",
    )
    .unwrap();

    let check = Command::new(bin())
        .arg("check")
        .arg(&src)
        .output()
        .expect("spawn resilient check");
    assert_eq!(
        check.status.code(),
        Some(0),
        "expected row-poly anonymous literal to typecheck; stdout={} stderr={}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
