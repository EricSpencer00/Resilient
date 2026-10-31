use std::{fs, process::Command};
fn check(name: &str, source: &str, success: bool) {
    let root = std::env::temp_dir().join(format!("try_purity_{}_{}", std::process::id(), name));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("input.rz");
    fs::write(&path, source).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rz"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("RESILIENT_") {
            command.env_remove(key);
        }
    }
    let output = command.arg("check").arg(&path).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    fs::remove_dir_all(root).unwrap();
    assert_eq!(output.status.success(), success, "{name}: {stderr}");
    if !success {
        assert!(
            stderr.contains("pure context")
                || stderr.contains("impure")
                || stderr.contains("io callback")
                || stderr.contains("cannot unify effect"),
            "{name}: {stderr}"
        );
    }
}

#[test]
fn pure_body_false() {
    check(
        "pure_body_False",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn f() { try { risky(); good(1); } catch Timeout {  } } f();"#,
        true,
    );
}

#[test]
fn pure_body_true() {
    check(
        "pure_body_True",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn f() { try { risky(); println(1); } catch Timeout {  } } f();"#,
        false,
    );
}

#[test]
fn pure_handler_false() {
    check(
        "pure_handler_False",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn f() { try { risky();  } catch Timeout { good(1); } } f();"#,
        true,
    );
}

#[test]
fn pure_handler_true() {
    check(
        "pure_handler_True",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn f() { try { risky();  } catch Timeout { println(1); } } f();"#,
        false,
    );
}

#[test]
fn attribute_pure_body_false() {
    check(
        "attribute_pure_body_False",
        r#"@pure fn risky() -> int fails Timeout { return 1; } @pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } @pure fn f() { try { risky(); good(1); } catch Timeout {  } } f();"#,
        true,
    );
}

#[test]
fn attribute_pure_body_true() {
    check(
        "attribute_pure_body_True",
        r#"@pure fn risky() -> int fails Timeout { return 1; } @pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } @pure fn f() { try { risky(); println(1); } catch Timeout {  } } f();"#,
        false,
    );
}

#[test]
fn attribute_pure_handler_false() {
    check(
        "attribute_pure_handler_False",
        r#"@pure fn risky() -> int fails Timeout { return 1; } @pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } @pure fn f() { try { risky();  } catch Timeout { good(1); } } f();"#,
        true,
    );
}

#[test]
fn attribute_pure_handler_true() {
    check(
        "attribute_pure_handler_True",
        r#"@pure fn risky() -> int fails Timeout { return 1; } @pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } @pure fn f() { try { risky();  } catch Timeout { println(1); } } f();"#,
        false,
    );
}

#[test]
fn invoked_good() {
    check(
        "invoked_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { try { risky(); f(1); } catch Timeout { } } run(good);"#,
        true,
    );
}

#[test]
fn invoked_noisy() {
    check(
        "invoked_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { try { risky(); f(1); } catch Timeout { } } run(noisy);"#,
        false,
    );
}

#[test]
fn handler_good() {
    check(
        "handler_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { try { risky(); } catch Timeout { f(1); } } run(good);"#,
        true,
    );
}

#[test]
fn handler_noisy() {
    check(
        "handler_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { try { risky(); } catch Timeout { f(1); } } run(noisy);"#,
        false,
    );
}

#[test]
fn passed_good() {
    check(
        "passed_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { f(1); } fn caller() { try { risky(); run(good); } catch Timeout { } } caller();"#,
        true,
    );
}

#[test]
fn passed_noisy() {
    check(
        "passed_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { f(1); } fn caller() { try { risky(); run(noisy); } catch Timeout { } } caller();"#,
        false,
    );
}

#[test]
fn unify_good() {
    check(
        "unify_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } fn combine(fn(int) -e-> int f, fn(int) -e-> int g) -> int { return f(1) + g(1); } fn caller() { try { risky(); combine(good, good); } catch Timeout { } } caller();"#,
        true,
    );
}

#[test]
fn unify_noisy() {
    check(
        "unify_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } fn combine(fn(int) -e-> int f, fn(int) -e-> int g) -> int { return f(1) + g(1); } fn caller() { try { risky(); combine(good, noisy); } catch Timeout { } } caller();"#,
        false,
    );
}

#[test]
fn alias_good() {
    check(
        "alias_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { f(1); } fn caller() { try { risky(); let alias = good; run(alias); } catch Timeout { } } caller();"#,
        true,
    );
}

#[test]
fn poly_good() {
    check(
        "poly_good",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } fn run(fn(int) -e-> int f) { try { risky(); good(1); f(1); } catch Timeout { } } pure fn caller() { run(good); } caller();"#,
        true,
    );
}

#[test]
fn alias_noisy() {
    check(
        "alias_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } pure fn run(fn(int) -> int f) { f(1); } fn caller() { try { risky(); let alias = noisy; run(alias); } catch Timeout { } } caller();"#,
        false,
    );
}

#[test]
fn poly_noisy() {
    check(
        "poly_noisy",
        r#"pure fn risky() -> int fails Timeout { return 1; } pure fn good(int x) -> int { return x; } io fn noisy(int x) -> int { println(x); return x; } fn run(fn(int) -e-> int f) { try { risky(); noisy(1); f(1); } catch Timeout { } } pure fn caller() { run(good); } caller();"#,
        false,
    );
}
