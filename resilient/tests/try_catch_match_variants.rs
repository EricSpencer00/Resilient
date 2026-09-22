use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

fn run_src(tag: &str, src: &str) -> (String, String, Option<i32>) {
    let mut path: PathBuf = std::env::temp_dir();
    path.push(format!("res_4636_{tag}_{}.rz", std::process::id()));
    {
        let mut file = std::fs::File::create(&path).expect("create temp source");
        file.write_all(src.as_bytes()).expect("write temp source");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--typecheck-strict"])
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
fn catch_accepts_failure_emitted_by_match_arm() {
    let src = "\
        fn risky(int value) fails Timeout { return value; }\n\
        fn main(int value) {\n\
            try {\n\
                match value {\n\
                    0 => risky(value),\n\
                    _ => 0,\n\
                };\n\
            } catch Timeout {\n\
                println(-1);\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, stderr, code) = run_src("match_arm", src);
    assert_eq!(
        code,
        Some(0),
        "match-arm failure should be catchable; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "strict typecheck should succeed; stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn catch_accepts_failure_emitted_by_match_guard() {
    let src = "\
        fn risky(int value) -> bool fails Timeout { return value > 0; }\n\
        fn main(int value) {\n\
            try {\n\
                match value {\n\
                    n if risky(n) => 0,\n\
                    _ => 1,\n\
                };\n\
            } catch Timeout {\n\
                println(-2);\n\
            }\n\
        }\n\
        main(1);\n\
    ";
    let (stdout, stderr, code) = run_src("match_guard", src);
    assert_eq!(
        code,
        Some(0),
        "match-guard failure should be catchable; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains("Program executed successfully"),
        "strict typecheck should succeed; stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn catch_still_rejects_unemitted_match_variant() {
    let src = "\
        fn safe(int value) { return value; }\n\
        fn main(int value) {\n\
            try {\n\
                match value {\n\
                    0 => safe(value),\n\
                    _ => 0,\n\
                };\n\
            } catch Timeout {\n\
                println(-3);\n\
            }\n\
        }\n\
        main(0);\n\
    ";
    let (stdout, stderr, code) = run_src("unemitted_match_variant", src);
    assert_ne!(
        code,
        Some(0),
        "unemitted catch variant must be rejected; stdout={stdout} stderr={stderr}"
    );
    assert!(
        stderr.contains("catch Timeout") || stdout.contains("catch Timeout"),
        "diagnostic should name the invalid catch; stdout={stdout} stderr={stderr}"
    );
}
