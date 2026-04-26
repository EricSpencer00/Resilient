//! RES-371: `--emit-live-log` writes one NDJSON record per live-block retry.

use std::io::Write;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn extract_u64_field<'a>(line: &'a str, key: &str) -> &'a str {
    let needle = format!("\"{}\":", key);
    let i = line
        .find(&needle)
        .unwrap_or_else(|| panic!("missing {key} in {line:?}"));
    let start = i + needle.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn emit_live_log_writes_two_retry_records() {
    let mut log_path = std::env::temp_dir();
    log_path.push(format!(
        "res371_emit_live_log_{}.ndjson",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&log_path);

    let mut src_path = std::env::temp_dir();
    src_path.push(format!("res371_emit_src_{}.rz", std::process::id()));

    let src = "\
static let fails_left = 2;\n\
\n\
fn maybe_fail() {\n\
    if fails_left > 0 {\n\
        fails_left = fails_left - 1;\n\
        assert(false, \"forced fail\");\n\
    }\n\
    return 42;\n\
}\n\
\n\
fn main(int _d) {\n\
    live {\n\
        let _r = maybe_fail();\n\
        println(\"ok\");\n\
    }\n\
}\n\
\n\
main(0);\n\
";

    {
        let mut f = std::fs::File::create(&src_path).expect("create temp source");
        f.write_all(src.as_bytes()).expect("write source");
    }

    let src_basename = src_path
        .file_name()
        .expect("basename")
        .to_str()
        .expect("utf8 basename")
        .to_string();

    let output = Command::new(bin())
        .arg("--emit-live-log")
        .arg(&log_path)
        .arg(&src_path)
        .output()
        .expect("spawn resilient");

    let _ = std::fs::remove_file(&src_path);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let log = std::fs::read_to_string(&log_path).expect("read log file");
    let _ = std::fs::remove_file(&log_path);

    let lines: Vec<&str> = log.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "expected 2 NDJSON lines for 2 retries before success; log:\n{log}"
    );

    let r0: usize = extract_u64_field(lines[0], "retry").parse().unwrap();
    let r1: usize = extract_u64_field(lines[1], "retry").parse().unwrap();
    assert_eq!(r0, 1);
    assert_eq!(r1, 2);

    assert!(
        lines[0].contains("\"block\":")
            && lines[0].contains(&src_basename)
            && lines[0].contains(".rz:"),
        "expected block label `basename:line`; line={}",
        lines[0]
    );
    assert!(
        lines[0].contains("forced fail"),
        "reason should include failure text; line={}",
        lines[0]
    );
    assert!(
        lines[0].contains("\"ts_ns\":") && lines[1].contains("\"ts_ns\":"),
        "ts_ns should be present; lines={lines:?}"
    );
}
