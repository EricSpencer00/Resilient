//! RES-4986: module impl blocks intentionally register legacy outer methods.
use sha2::{Digest, Sha256};
use std::{fs, io::Write, process::Command};

#[test]
fn inline_module_impl_remains_visible_on_outer_struct() {
    let source = "struct Cell { int value } mod m { impl Cell { fn f(self) -> int { return self.value; } } } let c = Cell { value: 7 }; println(c.f());";
    let path = std::env::temp_dir().join(format!("module_impl_compat_{}.rz", std::process::id()));
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
        .write_all(format!("MODULE_IMPL_COMPAT_RECORD {record}\n").as_bytes())
        .unwrap();
    fs::remove_file(path).unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines().rev();
    assert_eq!(
        lines.next(),
        Some("Program executed successfully"),
        "{stdout}"
    );
    assert_eq!(lines.next(), Some("7"), "{stdout}");
}
