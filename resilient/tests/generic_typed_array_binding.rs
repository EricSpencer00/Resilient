//! Regression coverage for RES-4538: generic bindings inside typed arrays.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch_file(source: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "res_generic_typed_array_binding_{}_{}.rz",
        std::process::id(),
        id
    ));
    std::fs::write(&path, source).expect("write generic typed-array fixture");
    path
}

fn typecheck(source: &str) -> Output {
    let path = scratch_file(source);
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--typecheck-strict"])
        .arg(&path)
        .output()
        .expect("spawn rz --typecheck-strict");
    let _ = std::fs::remove_file(path);
    output
}

#[test]
fn repeated_typed_array_bindings_reject_conflicting_elements() {
    let output = typecheck(
        "fn pair_last<T>(array<T> a, array<T> b) -> T { return b[0]; }\n\
         fn main() -> int { let x: int = pair_last([1], [\"bad\"]); return 0; }\n\
         main();\n",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "conflicting array<T> arguments were accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("type parameter `T`")
            || String::from_utf8_lossy(&output.stderr).contains("Type mismatch"),
        "diagnostic did not identify the generic mismatch: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn typed_array_binding_propagates_generic_return_type() {
    let output = typecheck(
        "fn head<T>(array<T> xs) -> T { return xs[0]; }\n\
         fn main() -> int { let x: int = head([1, 2]); return x; }\n\
         main();\n",
    );
    assert_eq!(
        output.status.code(),
        Some(0),
        "valid array<T> generic call was rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nested_typed_array_binding_rejects_conflicting_elements() {
    let output = typecheck(
        "fn nested<T>(array<array<T>> xs, array<array<T>> ys) -> T { return ys[0][0]; }\n\
         fn main() -> int { let x: int = nested([[1]], [[\"bad\"]]); return 0; }\n\
         main();\n",
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "conflicting nested array<T> arguments were accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
