//! RES-4986: strict frontend namespace resolution through the public CLI.
use sha2::{Digest, Sha256};
use std::{fs, io::Write, process::Command, sync::OnceLock};

fn run(
    case: &str,
    source: &str,
    library: Option<&str>,
    flags: &[&str],
    role: &str,
) -> std::process::Output {
    static IDENTITY: OnceLock<(String, String)> = OnceLock::new();
    let identity = IDENTITY.get_or_init(|| {
        let binary = env!("CARGO_BIN_EXE_rz");
        let version = Command::new(binary)
            .args(["--version", "--verbose"])
            .output()
            .unwrap();
        (
            format!("{:x}", Sha256::digest(fs::read(binary).unwrap())),
            String::from_utf8_lossy(&version.stdout).into_owned(),
        )
    });
    let root =
        std::env::temp_dir().join(format!("module_resolution_{}_{case}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
    if let Some(contents) = library {
        fs::write(root.join("lib.rz"), contents).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("--no-cache")
        .args(flags)
        .arg(&path)
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .env_remove("RESILIENT_RICH_DIAG")
        .env_remove("RESILIENT_CONST_FOLD")
        .env_remove("RESILIENT_INLINE")
        .env("RESILIENT_DISPATCH", "match")
        .output()
        .unwrap();
    // Emit source-bound observations in normal hosted test logs, including controls.
    let record = serde_json::json!({"case":case,"role":role,"source":source,"library":library,
        "flags":flags,"binary_sha256":identity.0,"version":identity.1,
        "dispatch":"match","overflow":"default","status":out.status.code(),
        "stdout":String::from_utf8_lossy(&out.stdout),"stderr":String::from_utf8_lossy(&out.stderr)});
    writeln!(std::io::stderr(), "MODULE_RESOLUTION_RECORD {record}").unwrap();
    fs::remove_dir_all(root).unwrap();
    out
}
fn positive(case: &str, source: &str, library: Option<&str>, expected: &str) {
    let out = run(
        case,
        source,
        library,
        &["--audit", "--vm"],
        "strict-runtime-acceptance",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), expected);
}
fn negative(case: &str, source: &str, library: Option<&str>, diagnostic: &str) {
    let out = run(
        case,
        source,
        library,
        &["--audit", "--vm"],
        "strict-negative-control",
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(diagnostic),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn qualified_inline_function_is_callable() {
    // These are diagnostic observations, not supported-runtime acceptance credit.
    for (name, source) in [
        (
            "sibling",
            "mod m { fn f(int x) -> int { return helper(x); } fn helper(int x) -> int { return x; } } println(m::f(7));",
        ),
        (
            "recursive",
            "mod m { fn f(int x) -> int { if x <= 0 { return 0; } return f(x - 1); } } println(m::f(2));",
        ),
        (
            "nested",
            "mod outer { mod inner { fn f(int x) -> int { return x; } } } println(outer::inner::f(7));",
        ),
    ] {
        run(
            name,
            source,
            None,
            &["--audit", "--vm"],
            "strict-backend-gap-observation",
        );
        run(
            &format!("{name}_backend"),
            source,
            None,
            &["--audit", "--vm", "--no-typecheck"],
            "backend-only-no-frontend-credit",
        );
    }
    positive(
        "qualified",
        "mod checks { fn f(int x) -> int { return x; } } println(checks::f(7));",
        None,
        "7",
    );
}
#[test]
fn module_members_do_not_leak_bare_bindings() {
    negative(
        "bare",
        "mod m { fn hidden(int x) -> int { return x; } } println(hidden(1));",
        None,
        "Undefined variable 'hidden'",
    );
}
#[test]
fn module_member_does_not_overwrite_outer_binding() {
    positive(
        "outer",
        "fn f(int x) -> int { return x + 1; } mod m { fn f(string x) -> string { return x; } } println(f(7)); println(m::f(\"ok\"));",
        None,
        "8\nok",
    );
}
#[test]
fn undefined_qualified_member_is_rejected() {
    negative(
        "missing",
        "mod m { fn f(int x) -> int { return x; } } println(m::missing(1));",
        None,
        "missing",
    );
}
#[test]
fn public_inline_glob_remains_callable() {
    positive(
        "glob",
        "mod m { pub fn f(int x) -> int { return x; } } use m::*; println(f(7));",
        None,
        "7",
    );
}
#[test]
fn private_inline_glob_member_does_not_leak() {
    negative(
        "private_glob",
        "mod m { pub fn f(int x) -> int { return x; } fn secret(int x) -> int { return x; } } use m::*; println(secret(1));",
        None,
        "Undefined variable 'secret'",
    );
}
#[test]
fn nested_public_glob_remains_callable() {
    positive(
        "nested_glob",
        "mod outer { mod inner { pub fn f(int x) -> int { return x; } } } use outer::inner::*; println(f(7));",
        None,
        "7",
    );
}
#[test]
fn file_import_preserves_siblings_and_local_shadowing() {
    positive(
        "file",
        "use \"lib.rz\" as lib; println(lib::public(7)); println(lib::shadow(7));",
        Some(
            "pub fn helper(int x) -> int { return x; } pub fn public(int x) -> int { return helper(x); } pub fn shadow(int x) -> int { let helper = fn(int y) -> int { return y + 2; }; return helper(x); }",
        ),
        "7\n9",
    );
}
#[test]
fn private_file_import_member_remains_unbound() {
    negative(
        "private_file",
        "use \"lib.rz\" as lib; println(lib::secret(1));",
        Some("pub fn f(int x) -> int { return x; } fn secret(int x) -> int { return x; }"),
        "Undefined variable 'lib::secret'",
    );
}
