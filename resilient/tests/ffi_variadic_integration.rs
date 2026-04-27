//! RES-316: end-to-end C variadic FFI smoke tests.

#![cfg(all(feature = "ffi", any(target_os = "linux", target_os = "macos")))]

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn resilient_bin() -> &'static str {
    env!("CARGO_BIN_EXE_rz")
}

fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn run_resilient_src(src: &str) -> (String, String, i32) {
    let tmp = std::env::temp_dir().join(format!(
        "res_ffi_variadic_{}_{}.rs",
        std::process::id(),
        next_seq()
    ));
    {
        let mut f = std::fs::File::create(&tmp).expect("create tmp file");
        f.write_all(src.as_bytes()).expect("write tmp file");
    }
    let output = Command::new(resilient_bin())
        .arg(&tmp)
        .output()
        .expect("failed to spawn resilient binary");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let code = output.status.code().unwrap_or(-1);
    let _ = std::fs::remove_file(&tmp);
    (stdout, stderr, code)
}

#[test]
fn printf_accepts_two_variadic_ints() {
    #[cfg(target_os = "macos")]
    let libc = "libSystem.dylib";
    #[cfg(target_os = "linux")]
    let libc = "libc.so.6";

    let src = format!(
        r#"extern "{lib}" {{ fn c_printf(fmt: String, ...) -> Int = "printf"; }};
fn main(int _d) {{
    c_printf("ffi variadic %lld %lld\n", 7, 35);
}}
main(0);"#,
        lib = libc
    );

    let (stdout, stderr, code) = run_resilient_src(&src);
    assert_eq!(code, 0, "stdout={stdout} stderr={stderr}");
    assert!(
        stdout.contains("ffi variadic 7 35"),
        "expected printf output, got stdout={stdout} stderr={stderr}"
    );
}
