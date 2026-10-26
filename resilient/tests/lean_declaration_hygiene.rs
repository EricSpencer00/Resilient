use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn emit(name: &str) -> String {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "lean_names_{}_{}.rz",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(
        &path,
        format!("fn {name}(int sample, bool eval) -> int {{ return sample + 1; }}"),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg(format!("--emit-lean-spec={name}"))
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn core_types_and_lean_keywords_use_generated_namespaces() {
    for name in [
        "Int",
        "Bool",
        "theorem",
        "namespace",
        "some",
        "Expr",
        "sample",
    ] {
        let theorem = emit(name);
        assert!(theorem.contains(&format!("namespace {name}_generated")));
        assert!(theorem.contains(&format!("def {name}_expr : Expr")));
        assert!(theorem.contains(&format!("theorem {name}_correct")));
        assert!(theorem.contains(&format!("_root_.{name}_generated.{name}_expr")));
    }
}

#[test]
fn generated_parameter_binders_are_independent_of_source_declarations() {
    let theorem = emit("lean_arg_0");
    assert!(theorem.contains("(lean_arg_0 : Int)"));
    assert!(theorem.contains("_root_.lean_arg_0_generated.lean_arg_0_expr"));
    assert!(theorem.contains("(\"sample\", lean_arg_0)"));
}

#[test]
fn source_names_with_generated_suffixes_remain_distinct() {
    let original = emit("sample");
    let suffixed = emit("sample_generated");
    assert!(original.contains("namespace sample_generated"));
    assert!(suffixed.contains("namespace sample_generated_generated"));
    assert!(suffixed.contains("_root_.sample_generated_generated.sample_generated_expr"));
}
