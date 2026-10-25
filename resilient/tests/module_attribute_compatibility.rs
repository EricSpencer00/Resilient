//! RES-4986: qualified declaration checks retain explicit overflow attributes.
#![cfg(feature = "z3")]
use sha2::{Digest, Sha256};
use std::{fs, io::Write, process::Command};

#[test]
fn inline_module_overflow_attribute_still_rejects_wraparound() {
    let source =
        "mod m { #[overflow_checked] fn overflow_f(int x) ensures x + 1 > x { return x; } }";
    let path =
        std::env::temp_dir().join(format!("module_attribute_compat_{}.rz", std::process::id()));
    fs::write(&path, source).unwrap();
    let binary = env!("CARGO_BIN_EXE_rz");
    let version = Command::new(binary)
        .args(["--version", "--verbose"])
        .output()
        .unwrap();
    assert!(version.status.success());
    let out = Command::new(binary)
        .args(["--no-cache", "--audit"])
        .arg(&path)
        .env_remove("RESILIENT_RICH_DIAG")
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .env_remove("RESILIENT_CONST_FOLD")
        .env_remove("RESILIENT_INLINE")
        .output()
        .unwrap();
    let record = serde_json::json!({"source":source,"flags":["--no-cache","--audit"],"backend":"treewalker","no_typecheck":false,
        "binary_sha256":format!("{:x}",Sha256::digest(fs::read(binary).unwrap())),"version_status":version.status.code(),
        "version":String::from_utf8_lossy(&version.stdout),"status":out.status.code(),"stdout":String::from_utf8_lossy(&out.stdout),"stderr":String::from_utf8_lossy(&out.stderr)});
    std::io::stderr()
        .lock()
        .write_all(format!("MODULE_ATTRIBUTE_COMPAT_RECORD {record}\n").as_bytes())
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("Type error:") && stderr.contains("LIA said safe, BV64 disproved it"),
        "{stderr}"
    );
}
