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
        "res_projection_where_{}_{}_{}",
        tag,
        std::process::id(),
        n
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

fn run_check(tag: &str, src: &str) -> std::process::Output {
    let dir = tmp_dir(tag);
    let src_path = dir.join("main.rz");
    std::fs::write(&src_path, src).expect("write test source");
    let output = Command::new(bin())
        .arg("check")
        .arg(&src_path)
        .output()
        .expect("spawn rz check");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

#[test]
fn projection_bound_accepts_let_bound_concrete_type() {
    let src = "trait Show { fn show(self) -> string; }\n\
trait Iter { type Item; fn next(self) -> int; }\n\
struct Num { int v }\n\
struct List { int n }\n\
impl Show for Num { fn show(self) -> string { return \"n\"; } }\n\
impl Iter for List {\n\
    type Item = Num;\n\
    fn next(self) -> int { return self.n; }\n\
}\n\
fn<I> collect(I it) where I::Item: Show { println(it.next()); }\n\
fn main(int _d) {\n\
    let it = new List { n: 0 };\n\
    collect(it);\n\
}\n\
main(0);\n";
    let output = run_check("ok", src);
    assert_eq!(
        output.status.code(),
        Some(0),
        "expected success; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn projection_bound_rejects_let_bound_violation() {
    let src = "trait Show { fn show(self) -> string; }\n\
trait Iter { type Item; fn next(self) -> int; }\n\
struct BadType { int x }\n\
struct List { int n }\n\
impl Iter for List {\n\
    type Item = BadType;\n\
    fn next(self) -> int { return self.n; }\n\
}\n\
fn<I> collect(I it) where I::Item: Show { println(it.next()); }\n\
fn main(int _d) {\n\
    let it = new List { n: 0 };\n\
    collect(it);\n\
}\n\
main(0);\n";
    let output = run_check("err", src);
    assert_eq!(
        output.status.code(),
        Some(1),
        "expected typecheck failure; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("associated type `List::Item` = `BadType` does not satisfy bound `Show`"),
        "unexpected stderr: {stderr}"
    );
}
