use std::process::{Command, Output};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_source(name: &str, attribute: &str) -> Output {
    let path = std::env::temp_dir().join(format!(
        "res_mmio_regmap_{}_{}.rz",
        std::process::id(),
        name
    ));
    let source = format!(
        "{attribute}\n\
         struct GPIOA {{ int mode, }}\n\
         fn main() -> int {{ return 0; }}\n\
         main();\n"
    );
    std::fs::write(&path, source).expect("write MMIO source");
    let output = Command::new(bin())
        .arg("--typecheck-strict")
        .arg(&path)
        .output()
        .expect("spawn rz");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn malformed_mmio_attributes_fail_from_source() {
    let cases = [
        (
            "invalid-base",
            r#"#[mmio(base = "0xZZZZ", size_bytes = "0x100")]"#,
            "base value is not a valid address",
        ),
        (
            "invalid-size",
            r#"#[mmio(base = "0x40010800", size_bytes = "0xGGGG")]"#,
            "size_bytes value is not a valid address",
        ),
        (
            "unquoted-base",
            r#"#[mmio(base = 0x40010800, size_bytes = "0x100")]"#,
            "base value must be quoted",
        ),
        (
            "missing-base",
            r#"#[mmio(size_bytes = "0x100")]"#,
            "requires a base argument",
        ),
        (
            "missing-size",
            r#"#[mmio(base = "0x40010800")]"#,
            "requires a size_bytes argument",
        ),
        (
            "zero-base",
            r#"#[mmio(base = "0x0", size_bytes = "0x100")]"#,
            "base address 0x0 is reserved",
        ),
        (
            "unknown-key",
            r#"#[mmio(base = "0x40010800", size = "0x100")]"#,
            "unknown argument 'size'",
        ),
    ];

    for (name, attribute, expected) in cases {
        let output = run_source(name, attribute);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{name} unexpectedly passed:\nstdout={}\nstderr={stderr}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            stderr.contains("GPIOA"),
            "{name} diagnostic should name the struct: {stderr}"
        );
        assert!(
            stderr.contains(expected),
            "{name} diagnostic missing {expected:?}: {stderr}"
        );
    }
}

#[test]
fn valid_mmio_attribute_still_typechecks() {
    let output = run_source(
        "valid",
        r#"#[mmio(base = "0x40010800", size_bytes = "0x400")]"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "valid MMIO attribute failed:\nstdout={stdout}\nstderr={stderr}"
    );
}
