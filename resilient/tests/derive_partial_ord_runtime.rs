use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_src(tag: &str, src: &str) -> (String, String, Option<i32>) {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("res_derive_{tag}_{}.rz", std::process::id()));
    {
        let mut f = std::fs::File::create(&path).expect("create temp .rz");
        f.write_all(src.as_bytes()).expect("write src");
    }
    let output = Command::new(bin())
        .arg(&path)
        .output()
        .expect("spawn resilient");
    let _ = std::fs::remove_file(&path);
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn partial_ord_struct_comparison() {
    let src = r#"
struct Point { int x, int y }

fn main() {
    let a = new Point { x: 1, y: 2 };
    let b = new Point { x: 2, y: 0 };
    if a == a {
        println("eq ok");
    }
    if a != b {
        println("neq ok");
    }
}

main();
"#;
    let (stdout, stderr, code) = run_src("partial_eq", src);
    assert_eq!(
        code,
        Some(0),
        "struct equality check must exit 0; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("eq ok"),
        "expected reflexive equality; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("neq ok"),
        "expected struct inequality; stdout={stdout} stderr={stderr}"
    );
}
