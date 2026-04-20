//! FFI Phase 1 Task 9: compile the C test helper into a shared library
//! whose path is injected into the integration tests via an env var.
//!
//! Only runs when the `ffi` feature is active, so the default build
//! has zero C-toolchain dependency. Uses the system `cc` directly
//! rather than the `cc` crate, because `cc::Build::shared_flag(true)`
//! still produces a static archive on current versions.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Only compile the FFI test helper when the `ffi` feature is active.
    if std::env::var("CARGO_FEATURE_FFI").is_err() {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let c_src = PathBuf::from("tests/ffi/lib_testhelper.c");

    // Tell Cargo to re-run build.rs if the C source changes.
    println!("cargo:rerun-if-changed=tests/ffi/lib_testhelper.c");

    let (lib_name, extra_flags): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("libtesthelper.dylib", &["-dynamiclib"])
    } else {
        ("libtesthelper.so", &["-shared", "-fPIC"])
    };

    let lib_path = out_dir.join(lib_name);

    let status = Command::new("cc")
        .args(extra_flags)
        .arg("-o")
        .arg(&lib_path)
        .arg(&c_src)
        .status()
        .expect("Failed to run C compiler — ensure `cc` is in PATH");

    if !status.success() {
        panic!("C compiler failed building FFI test helper");
    }

    // Export the path so integration tests can find the library.
    println!(
        "cargo:rustc-env=RESILIENT_FFI_TESTHELPER_PATH={}",
        lib_path.display()
    );
}
