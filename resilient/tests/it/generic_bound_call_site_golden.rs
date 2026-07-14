//! RES-4048 (A-E2 first increment): golden coverage for enforcing
//! `<T: Trait>` bound satisfaction at generic call sites, beyond the
//! direct-struct-literal-argument case `traits.rs` already covered.
//!
//! Two forms previously slipped past typecheck entirely and only
//! surfaced as a confusing runtime "no such field" error deep inside
//! the generic body:
//!
//!   1. the argument is a call to a fn with a known, concrete
//!      return-type annotation (`render(make_rock())`)
//!   2. the argument is a plainly-typed parameter forwarded straight
//!      through (`fn wrapper(Rock r) -> int { render(r) }`)
//!
//! Both now resolve to a concrete type at the call site and are
//! checked against the declared bound just like a struct literal.

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
        "res_generic_bound_call_site_golden_{}_{}_{}",
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

/// ACCEPT: the bounded type param is satisfied both when the argument
/// is a call-expression with a concrete return type and when it's a
/// plainly-typed parameter forwarded through — neither form should
/// newly regress now that both are resolved and checked.
#[test]
fn generic_bound_satisfied_via_call_result_and_param_forward_matches_golden() {
    let src = "trait Drawable { fn draw(self) -> int; }\n\
struct Circle { int radius }\n\
impl Drawable for Circle { fn draw(self) -> int { return self.radius; } }\n\
fn render<T: Drawable>(T shape) -> int { return shape.draw(); }\n\
fn make_circle() -> Circle { return new Circle { radius: 4 }; }\n\
fn render_param(Circle c) -> int { return render(c); }\n\
fn main(int _d) {\n\
    println(render(make_circle()));\n\
    println(render_param(new Circle { radius: 9 }));\n\
}\n\
main(0);\n";
    assert_eq!(
        run_check("ok", true, src),
        read_expected("generic_bound_call_site_ok.txt")
    );
}

/// REJECT: `Rock` has no `impl Drawable for Rock`. Previously this
/// slipped past typecheck because the argument to `render` is a
/// `CallExpression` (`make_rock()`), not a struct literal — the
/// unsatisfied bound only surfaced as a runtime "no such field 'draw'"
/// error. It must now be a clean typecheck-time diagnostic.
#[test]
fn generic_bound_violated_via_call_result_rejected_matches_golden() {
    let src = "trait Drawable { fn draw(self) -> int; }\n\
struct Rock { int weight }\n\
fn render<T: Drawable>(T shape) -> int { return shape.draw(); }\n\
fn make_rock() -> Rock { return new Rock { weight: 4 }; }\n\
fn main(int _d) {\n\
    println(render(make_rock()));\n\
}\n\
main(0);\n";
    assert_eq!(
        run_check("err", false, src),
        read_expected("generic_bound_call_site_err.txt")
    );
}

/// REJECT: same unsatisfied bound, but the argument is a plainly-typed
/// parameter (`Rock r`) forwarded straight through a wrapper fn rather
/// than a fresh struct literal or `let`-bound name.
#[test]
fn generic_bound_violated_via_param_forward_rejected_matches_golden() {
    let src = "trait Drawable { fn draw(self) -> int; }\n\
struct Rock { int weight }\n\
fn render<T: Drawable>(T shape) -> int { return shape.draw(); }\n\
fn wrapper(Rock r) -> int { return render(r); }\n\
fn main(int _d) {\n\
    println(wrapper(new Rock { weight: 4 }));\n\
}\n\
main(0);\n";
    assert_eq!(
        run_check("err_param", false, src),
        read_expected("generic_bound_call_site_param_err.txt")
    );
}
