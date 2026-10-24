//! Comment text must not supply or hide an external refinement action.
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn check(spec: &str, accepted: bool) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "res_tla_comments_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let source = root.join("program.rz");
    std::fs::write(
        &source,
        "@refines(spec = \"counter.tla\", action = \"Inc\")\nfn increment(int value) { return value; }",
    )
    .unwrap();
    std::fs::write(root.join("counter.tla"), spec).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .arg("-t")
        .arg(&source)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(output.status.success(), accepted, "{output:?}");
    if !accepted {
        assert!(String::from_utf8_lossy(&output.stderr).contains("does not define action"));
    }
}

#[test]
fn nested_comment_does_not_define_action() {
    check("---- MODULE Counter ----\n(* outer\n(* inner *)\nInc == TRUE\n*)\n====", false);
}

#[test]
fn line_comment_cannot_open_block_comment() {
    check("---- MODULE Counter ----\n\\* (* ignored\nInc == TRUE\n====", true);
}
