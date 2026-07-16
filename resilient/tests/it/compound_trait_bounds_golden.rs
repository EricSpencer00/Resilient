//! RES-4078 (A-E2): golden coverage for compound trait bounds —
//! `fn render<T: TraitA + TraitB>` must require the concrete type
//! argument to satisfy EVERY listed trait. Parsing (RES-290) and
//! all-bounds enforcement (traits.rs Pass 3) predate this ticket, but
//! the compound case had zero golden coverage: nothing pinned down
//! that a type satisfying only ONE of two bounds is rejected naming
//! the missing trait.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn tmp_dir(tag: &str) -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "res_compound_trait_bounds_golden_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn normalize_output(path: &Path, output: std::process::Output, include_streams: bool) -> String {
    if !include_streams {
        return format!("exit={}\n", output.status.code().unwrap_or(-1));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let path_str = path.to_string_lossy();
    let combined = format!(
        "exit={}\n{}{}",
        output.status.code().unwrap_or(-1),
        stdout,
        stderr
    );
    combined.replace(path_str.as_ref(), "<tmp>.rz")
}

fn run_check(tag: &str, quiet: bool, src: &str) -> String {
    let dir = tmp_dir(tag);
    let src_path = dir.join("main.rz");
    std::fs::write(&src_path, src).expect("write test source");
    let mut cmd = Command::new(bin());
    cmd.arg("check");
    if quiet {
        cmd.arg("-q");
    }
    let output = cmd.arg(&src_path).output().expect("spawn rz check");
    let normalized = normalize_output(&src_path, output, !quiet);
    let _ = std::fs::remove_dir_all(&dir);
    normalized
}

fn read_expected(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(name);
    std::fs::read_to_string(path).expect("read golden")
}

const COMMON: &str = "trait Drawable { fn draw(self) -> int; }\n\
trait Sizable { fn size(self) -> int; }\n\
struct Circle { int radius }\n\
fn render<T: Drawable + Sizable>(T shape) -> int { let d: int = shape.draw(); let s: int = shape.size(); return d + s; }\n";

/// ACCEPT: `Circle` implements both `Drawable` and `Sizable`, so it
/// satisfies the full compound bound.
#[test]
fn compound_bound_both_impls_satisfied_matches_golden() {
    let src = format!(
        "{COMMON}impl Drawable for Circle {{ fn draw(self) -> int {{ return self.radius; }} }}\n\
impl Sizable for Circle {{ fn size(self) -> int {{ return self.radius * 2; }} }}\n\
fn main(int _d) {{ println(render(new Circle {{ radius: 4 }})); }}\n\
main(0);\n"
    );
    assert_eq!(
        run_check("ok", true, &src),
        read_expected("compound_trait_bounds_ok.txt")
    );
}

/// REJECT: `Circle` implements `Drawable` but NOT `Sizable` — the
/// diagnostic must name the second (missing) trait of the compound
/// bound, proving enforcement doesn't stop at the first bound.
#[test]
fn compound_bound_missing_second_trait_rejected_matches_golden() {
    let src = format!(
        "{COMMON}impl Drawable for Circle {{ fn draw(self) -> int {{ return self.radius; }} }}\n\
fn main(int _d) {{ println(render(new Circle {{ radius: 4 }})); }}\n\
main(0);\n"
    );
    assert_eq!(
        run_check("err_second", false, &src),
        read_expected("compound_trait_bounds_missing_second_err.txt")
    );
}

/// REJECT: the mirror case — `Circle` implements `Sizable` but NOT
/// `Drawable`, so the FIRST trait of the compound bound is the one
/// named in the diagnostic.
#[test]
fn compound_bound_missing_first_trait_rejected_matches_golden() {
    let src = format!(
        "{COMMON}impl Sizable for Circle {{ fn size(self) -> int {{ return self.radius * 2; }} }}\n\
fn main(int _d) {{ println(render(new Circle {{ radius: 4 }})); }}\n\
main(0);\n"
    );
    assert_eq!(
        run_check("err_first", false, &src),
        read_expected("compound_trait_bounds_missing_first_err.txt")
    );
}

/// RES-4087: `COMMON` above binds each method call to an intermediate
/// `let x: int` before summing — that was the documented *workaround*
/// for a checker bug where `shape.draw() + shape.size()` (summed
/// directly, no intermediate bindings) was misinferred as array concat
/// and rejected with "declared int, returning array" even though the
/// program runs fine and prints 12. This test pins the workaround-free
/// form so the bug can't silently come back.
#[test]
fn compound_bound_direct_method_call_sum_not_misinferred_as_array() {
    let src = "trait Drawable { fn draw(self) -> int; }\n\
trait Sizable { fn size(self) -> int; }\n\
struct Circle { int radius }\n\
impl Drawable for Circle { fn draw(self) -> int { return self.radius; } }\n\
impl Sizable for Circle { fn size(self) -> int { return self.radius * 2; } }\n\
fn render<T: Drawable + Sizable>(T shape) -> int { return shape.draw() + shape.size(); }\n\
fn main(int _d) { println(render(new Circle { radius: 4 })); }\n\
main(0);\n";
    assert_eq!(
        run_check("direct_sum_ok", true, src),
        read_expected("compound_trait_bounds_ok.txt")
    );
}
