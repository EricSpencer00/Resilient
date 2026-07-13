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
        "res_projection_where_golden_{}_{}_{}",
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

#[test]
fn projection_where_clause_valid_check_output_matches_golden() {
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
    assert_eq!(
        run_check("ok", true, src),
        read_expected("projection_where_clause_ok.txt")
    );
}

#[test]
fn projection_where_clause_invalid_check_output_matches_golden() {
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
    assert_eq!(
        run_check("err", false, src),
        read_expected("projection_where_clause_err.txt")
    );
}
