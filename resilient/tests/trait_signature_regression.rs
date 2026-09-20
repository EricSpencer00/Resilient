//! Regression coverage for trait implementation signatures.
//!
//! A trait return annotation is part of the contract that generic dispatch
//! relies on. An implementation that omits that annotation must not silently
//! bypass the contract and defer the mismatch to runtime.

use std::sync::atomic::{AtomicUsize, Ordering};

fn run_check(tag: &str, source: &str) -> (bool, String) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "res_trait_signature_{}_{}_{}",
        tag,
        std::process::id(),
        id
    ));
    std::fs::create_dir_all(&dir).expect("create temporary source directory");
    let source_path = dir.join("main.rz");
    std::fs::write(&source_path, source).expect("write temporary source");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["check"])
        .arg(&source_path)
        .output()
        .expect("run rz check");
    let mut diagnostics = String::from_utf8_lossy(&output.stderr).into_owned();
    diagnostics.push_str(&String::from_utf8_lossy(&output.stdout));
    let _ = std::fs::remove_dir_all(&dir);
    (output.status.success(), diagnostics)
}

#[test]
fn omitted_trait_impl_return_type_is_rejected() {
    let source = r#"
trait Render { fn render(self) -> string; }
struct Box { int value }
impl Render for Box {
    fn render(self) { return self.value; }
}
fn use_render<T: Render>(T item) -> string { return item.render(); }
fn main(int _d) { println(use_render(new Box { value: 3 })); }
main(0);
"#;
    let (success, diagnostics) = run_check("omitted", source);
    assert!(
        !success,
        "omitted return type unexpectedly passed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("implementation omits a return type"),
        "missing trait return-type diagnostic: {diagnostics}"
    );
}

#[test]
fn compatible_trait_impl_return_type_is_accepted() {
    let source = r#"
trait Render { fn render(self) -> string; }
struct Box { int value }
impl Render for Box { fn render(self) -> string { return "ok"; } }
fn use_render<T: Render>(T item) -> string { return item.render(); }
fn main(int _d) { println(use_render(new Box { value: 3 })); }
main(0);
"#;
    let (success, diagnostics) = run_check("compatible", source);
    assert!(
        success,
        "compatible return type was rejected: {diagnostics}"
    );
}

#[test]
fn incompatible_trait_impl_return_type_remains_rejected() {
    let source = r#"
trait Render { fn render(self) -> string; }
struct Box { int value }
impl Render for Box { fn render(self) -> int { return self.value; } }
fn use_render<T: Render>(T item) -> string { return item.render(); }
fn main(int _d) { println(use_render(new Box { value: 3 })); }
main(0);
"#;
    let (success, diagnostics) = run_check("incompatible", source);
    assert!(
        !success,
        "incompatible return type unexpectedly passed: {diagnostics}"
    );
    assert!(
        diagnostics.contains("return type") && diagnostics.contains("string"),
        "missing incompatible return-type diagnostic: {diagnostics}"
    );
}
