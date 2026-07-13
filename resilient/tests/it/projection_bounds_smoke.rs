//! Smoke tests for projection-bound checking when a struct literal is
//! stored in a local binding before being passed to a generic function.

use std::io::Write;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn run_resilient_src(src: &str) -> (String, String, i32) {
    let tmp = std::env::temp_dir().join(format!(
        "res_projection_bounds_{}_{}.rz",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock went backwards")
            .as_nanos()
    ));
    {
        let mut file = std::fs::File::create(&tmp).expect("create temp file");
        file.write_all(src.as_bytes()).expect("write temp file");
    }
    let output = Command::new(bin())
        .arg(&tmp)
        .output()
        .expect("spawn resilient binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_file(&tmp);
    (stdout, stderr, code)
}

fn projection_bound_program(arg_expr: &str) -> String {
    format!(
        r#"
trait Show {{
    fn show(self) -> string;
}}

trait Iter {{
    type Item;
    fn next(self) -> int;
}}

struct Num {{ int v }}
struct List {{ int n }}

impl Show for Num {{
    fn show(self) -> string {{ return "n"; }}
}}

impl Iter for List {{
    type Item = Num;
    fn next(self) -> int {{ return self.n; }}
}}

fn<I> collect(I it) where I::Item: Show {{
    println(it.next());
}}

fn main(int _d) {{
    let list = new List {{ n: 0 }};
    let alias = list;
    collect({arg_expr});
}}

main(0);
"#
    )
}

#[test]
fn projection_bound_survives_simple_let_binding() {
    let src = projection_bound_program("list");
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line.trim() == "0"),
        "expected the program to typecheck and run, got stdout={stdout} stderr={stderr}"
    );
}

#[test]
fn projection_bound_survives_identifier_alias() {
    let src = projection_bound_program("alias");
    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.lines().any(|line| line.trim() == "0"),
        "expected the program to typecheck and run, got stdout={stdout} stderr={stderr}"
    );
}
