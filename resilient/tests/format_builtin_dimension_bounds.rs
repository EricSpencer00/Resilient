use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_check(source: &str) -> Output {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path: PathBuf = std::env::temp_dir().join(format!(
        "res4824_format_dimension_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write format fixture");
    let output = Command::new(bin())
        .arg("check")
        .arg(&path)
        .output()
        .expect("run format check");
    let _ = std::fs::remove_file(path);
    output
}

fn check_spec(spec: &str, ty: &str) -> Output {
    let template = format!("{{:{spec}}}");
    let source = format!(
        "#[format_builtin(template = \"{template}\", args = 1)]\nfn fmt({ty} x) -> {ty} {{ return x; }}\n"
    );
    run_check(&source)
}

fn diagnostics(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn accepts_legacy_width_and_precision_at_limit() {
    for (spec, ty) in [("65535d", "int"), (".65535f", "float")] {
        let output = check_spec(spec, ty);
        assert!(
            output.status.success(),
            "boundary format spec {spec} failed: {}",
            diagnostics(&output)
        );
    }
}

#[test]
fn rejects_legacy_integer_width_above_limit() {
    let output = check_spec("65536d", "int");
    let diagnostics = diagnostics(&output);
    assert!(
        diagnostics.contains("integer width 65536 exceeds maximum 65535"),
        "oversized integer width was not rejected: {diagnostics}"
    );
}

#[test]
fn rejects_legacy_float_precision_above_limit() {
    let output = check_spec(".65536f", "float");
    let diagnostics = diagnostics(&output);
    assert!(
        diagnostics.contains("float precision 65536 exceeds maximum 65535"),
        "oversized float precision was not rejected: {diagnostics}"
    );
}
