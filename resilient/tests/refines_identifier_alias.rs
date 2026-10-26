//! CLI regression coverage for identifier aliases on refined extern paths.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "refines-alias-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create fixture");
        std::fs::write(
            path.join("counter.tla"),
            "---- MODULE counter ----\nInc == TRUE\n====\n",
        )
        .expect("write spec");
        Self(path)
    }

    fn run(&self, source: &str) -> std::process::Output {
        let input = self.0.join("input.rz");
        std::fs::write(&input, source).expect("write source");
        Command::new(env!("CARGO_BIN_EXE_rz"))
            .args(["--typecheck", "--no-cache"])
            .arg(input)
            .output()
            .expect("run compiler")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn refined_alias_cannot_bypass_extern_contract_requirement() {
    let fixture = Fixture::new();
    let output = fixture.run(
        "@refines(spec = \"counter.tla\", action = \"Inc\")\nfn refined(int value) { let call = foreign; return call(value); }\nextern \"counter\" { fn foreign(value: Int) -> Int; };",
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "uncontracted alias was accepted");
    assert!(stderr.contains("error[refinement]"), "{stderr}");
    assert!(stderr.contains("extern fn `foreign`"), "{stderr}");
    assert!(stderr.contains("input.rz:2:"), "{stderr}");
}

#[test]
fn refined_fully_contracted_alias_remains_accepted() {
    let output = Fixture::new().run(
        "@refines(spec = \"counter.tla\", action = \"Inc\")\nfn refined(int value) { let call = foreign; return call(value); }\nextern \"counter\" { fn foreign(value: Int) -> Int requires value >= 0 ensures result >= 0; };",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn non_refined_alias_remains_permissive() {
    let output = Fixture::new().run(
        "fn plain(int value) { let call = foreign; return call(value); }\nextern \"counter\" { fn foreign(value: Int) -> Int; };",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
