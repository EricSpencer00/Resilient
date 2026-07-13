//! RES-340: end-to-end smoke for `RESILIENT_RICH_DIAG=1`.
//!
//! Subprocess-isolated so the env var doesn't leak between tests.
//! Default behaviour (no env var) is exercised by the in-tree unit
//! tests in `typechecker::res340_rich_type_mismatch_tests`; this
//! file covers the gate flipping the format on at the CLI seam.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res340_rich_diag_smoke_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

const BAD_SRC: &str = "fn drive(int dist) { return dist; }\nfn main() { let r = drive(\"oops\"); return r; }\nmain();\n";

#[test]
fn rich_diag_off_by_default_uses_legacy_format() {
    let dir = tmp_dir("default");
    let src = dir.join("bad.rz");
    std::fs::write(&src, BAD_SRC).unwrap();

    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        // Explicitly clear any inherited setting.
        .env_remove("RESILIENT_RICH_DIAG")
        .output()
        .expect("spawn resilient check");

    assert_eq!(
        output.status.code(),
        Some(1),
        "type error must exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Type mismatch in argument 1: expected int, got string"),
        "default format should be the legacy short message; stderr={stderr}"
    );
    assert!(
        !stderr.contains("error[E0007]"),
        "rich format must NOT appear by default; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rich_diag_env_var_enables_rustc_style_block() {
    let dir = tmp_dir("rich");
    let src = dir.join("bad.rz");
    std::fs::write(&src, BAD_SRC).unwrap();

    let output = Command::new(bin())
        .arg("check")
        .arg(&src)
        .env("RESILIENT_RICH_DIAG", "1")
        .output()
        .expect("spawn resilient check with rich diag");

    assert_eq!(
        output.status.code(),
        Some(1),
        "type error must still exit 1; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("error[E0007]: type mismatch in argument 1"),
        "rich header must appear; stderr={stderr}"
    );
    assert!(
        stderr.contains("expected `int`"),
        "rich expected-type label missing; stderr={stderr}"
    );
    assert!(
        stderr.contains("found `string`"),
        "rich found-type label missing; stderr={stderr}"
    );
    assert!(
        stderr.contains("note: expected `int` because of this declaration"),
        "secondary note pointing at the declaration is missing; stderr={stderr}"
    );
    // Secondary span renders the fn declaration line in the snippet.
    assert!(
        stderr.contains("fn drive(int dist)"),
        "declaration snippet missing from rich output; stderr={stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
