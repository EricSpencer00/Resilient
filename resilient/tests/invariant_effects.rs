use std::{fs, process::Command};
fn check(name: &str, source: &str, success: bool) {
    let root =
        std::env::temp_dir().join(format!("invariant_effect_{}_{}", std::process::id(), name));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
    let mut results = Vec::new();
    for mode in ["check", "interpreter", "match", "direct"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("RESILIENT_") {
                command.env_remove(key);
            }
        }
        if mode == "check" {
            command.arg("check");
        } else {
            command.args(["--typecheck", "--no-cache"]);
        }
        command.arg(&path);
        if mode == "match" || mode == "direct" {
            command.arg("--vm").env("RESILIENT_DISPATCH", mode);
        }
        results.push((mode, command.output().unwrap()));
    }
    fs::remove_dir_all(root).unwrap();
    for (mode, output) in results {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            output.status.success(),
            success,
            "{name}/{mode}: {stdout}\n{stderr}"
        );
        assert!(
            !stdout.lines().any(|line| line == "0" || line == "1"),
            "{name}/{mode}: forbidden I/O: {stdout}"
        );
        if !success {
            assert!(
                stderr.contains("pure context")
                    || stderr.contains("impure")
                    || stderr.contains("calls unannotated fn")
                    || stderr.contains("io callback")
                    || stderr.contains("cannot unify effect"),
                "{name}/{mode}: {stderr}"
            );
        }
    }
}

#[test]
fn pure_while_good() {
    check(
        "pure_while_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { let i=0; while i<1 invariant good(i) { i=i+1; } } run();"#,
        true,
    );
}

#[test]
fn pure_while_noisy() {
    check(
        "pure_while_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { let i=0; while i<1 invariant noisy(i) { i=i+1; } } run();"#,
        false,
    );
}

#[test]
fn pure_for_good() {
    check(
        "pure_for_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { for i in [1] invariant good(i) { } } run();"#,
        true,
    );
}

#[test]
fn pure_for_noisy() {
    check(
        "pure_for_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { for i in [1] invariant noisy(i) { } } run();"#,
        false,
    );
}

#[test]
fn pure_body_good() {
    check(
        "pure_body_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { for i in [1] { invariant good(i); } } run();"#,
        true,
    );
}

#[test]
fn pure_body_noisy() {
    check(
        "pure_body_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run() { for i in [1] { invariant noisy(i); } } run();"#,
        false,
    );
}

#[test]
fn attribute_pure_while_good() {
    check(
        "attribute_pure_while_good",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { let i=0; while i<1 invariant good(i) { i=i+1; } } run();"#,
        true,
    );
}

#[test]
fn attribute_pure_while_noisy() {
    check(
        "attribute_pure_while_noisy",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { let i=0; while i<1 invariant noisy(i) { i=i+1; } } run();"#,
        false,
    );
}

#[test]
fn attribute_pure_for_good() {
    check(
        "attribute_pure_for_good",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { for i in [1] invariant good(i) { } } run();"#,
        true,
    );
}

#[test]
fn attribute_pure_for_noisy() {
    check(
        "attribute_pure_for_noisy",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { for i in [1] invariant noisy(i) { } } run();"#,
        false,
    );
}

#[test]
fn attribute_pure_body_good() {
    check(
        "attribute_pure_body_good",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { for i in [1] { invariant good(i); } } run();"#,
        true,
    );
}

#[test]
fn attribute_pure_body_noisy() {
    check(
        "attribute_pure_body_noisy",
        r#"@pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } @pure fn run() { for i in [1] { invariant noisy(i); } } run();"#,
        false,
    );
}

#[test]
fn callback_while_good() {
    check(
        "callback_while_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { let i=0; while i<1 invariant f(i) { i=i+1; } } run(good);"#,
        true,
    );
}

#[test]
fn polymorphic_while_good() {
    check(
        "polymorphic_while_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { let i=0; while i<1 invariant good(i) && f(i) { i=i+1; } } pure fn caller() { run(good); } caller();"#,
        true,
    );
}

#[test]
fn callback_while_noisy() {
    check(
        "callback_while_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { let i=0; while i<1 invariant f(i) { i=i+1; } } run(noisy);"#,
        false,
    );
}

#[test]
fn polymorphic_while_noisy() {
    check(
        "polymorphic_while_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { let i=0; while i<1 invariant noisy(i) && f(i) { i=i+1; } } pure fn caller() { run(good); } caller();"#,
        false,
    );
}

#[test]
fn callback_for_good() {
    check(
        "callback_for_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { for i in [1] invariant f(i) { } } run(good);"#,
        true,
    );
}

#[test]
fn polymorphic_for_good() {
    check(
        "polymorphic_for_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { for i in [1] invariant good(i) && f(i) { } } pure fn caller() { run(good); } caller();"#,
        true,
    );
}

#[test]
fn callback_for_noisy() {
    check(
        "callback_for_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { for i in [1] invariant f(i) { } } run(noisy);"#,
        false,
    );
}

#[test]
fn polymorphic_for_noisy() {
    check(
        "polymorphic_for_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { for i in [1] invariant noisy(i) && f(i) { } } pure fn caller() { run(good); } caller();"#,
        false,
    );
}

#[test]
fn callback_body_good() {
    check(
        "callback_body_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { for i in [1] { invariant f(i); } } run(good);"#,
        true,
    );
}

#[test]
fn polymorphic_body_good() {
    check(
        "polymorphic_body_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { for i in [1] { invariant good(i) && f(i); } } pure fn caller() { run(good); } caller();"#,
        true,
    );
}

#[test]
fn callback_body_noisy() {
    check(
        "callback_body_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn run(fn(int) -> bool f) { for i in [1] { invariant f(i); } } run(noisy);"#,
        false,
    );
}

#[test]
fn polymorphic_body_noisy() {
    check(
        "polymorphic_body_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn run(fn(int) -e-> bool f) { for i in [1] { invariant noisy(i) && f(i); } } pure fn caller() { run(good); } caller();"#,
        false,
    );
}

#[test]
fn callsite_while_good() {
    check(
        "callsite_while_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { let i=0; while i<1 invariant apply(good,i) { i=i+1; } } caller();"#,
        true,
    );
}

#[test]
fn unify_while_good() {
    check(
        "unify_while_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { let i=0; while i<1 invariant combine(good,good,i) { i=i+1; } } caller();"#,
        true,
    );
}

#[test]
fn callsite_while_noisy() {
    check(
        "callsite_while_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { let i=0; while i<1 invariant apply(noisy,i) { i=i+1; } } caller();"#,
        false,
    );
}

#[test]
fn unify_while_noisy() {
    check(
        "unify_while_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { let i=0; while i<1 invariant combine(good,noisy,i) { i=i+1; } } caller();"#,
        false,
    );
}

#[test]
fn callsite_for_good() {
    check(
        "callsite_for_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { for i in [1] invariant apply(good,i) { } } caller();"#,
        true,
    );
}

#[test]
fn unify_for_good() {
    check(
        "unify_for_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { for i in [1] invariant combine(good,good,i) { } } caller();"#,
        true,
    );
}

#[test]
fn callsite_for_noisy() {
    check(
        "callsite_for_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { for i in [1] invariant apply(noisy,i) { } } caller();"#,
        false,
    );
}

#[test]
fn unify_for_noisy() {
    check(
        "unify_for_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { for i in [1] invariant combine(good,noisy,i) { } } caller();"#,
        false,
    );
}

#[test]
fn callsite_body_good() {
    check(
        "callsite_body_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { for i in [1] { invariant apply(good,i); } } caller();"#,
        true,
    );
}

#[test]
fn unify_body_good() {
    check(
        "unify_body_good",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { for i in [1] { invariant combine(good,good,i); } } caller();"#,
        true,
    );
}

#[test]
fn callsite_body_noisy() {
    check(
        "callsite_body_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } pure fn apply(fn(int) -> bool f, int x) -> bool { return f(x); } fn caller() { for i in [1] { invariant apply(noisy,i); } } caller();"#,
        false,
    );
}

#[test]
fn unify_body_noisy() {
    check(
        "unify_body_noisy",
        r#"pure fn good(int x) -> bool { return x>=0; } io fn noisy(int x) -> bool { println(x); return true; } fn combine(fn(int) -e-> bool f, fn(int) -e-> bool g, int x) -> bool { return f(x) && g(x); } fn caller() { for i in [1] { invariant combine(good,noisy,i); } } caller();"#,
        false,
    );
}
