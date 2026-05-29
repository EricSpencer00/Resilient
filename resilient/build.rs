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
    // RES-1248: declare the build script's actual dependencies so Cargo
    // doesn't re-run it unnecessarily. Without these directives Cargo
    // falls back to its default heuristic of re-running build.rs on
    // every package change, which on a 55k-line `lib.rs` happens on
    // every incremental rebuild.
    //
    // We list:
    //   - the build script itself (so edits here invalidate the cache),
    //   - the C source we sometimes compile (only matters under `ffi`),
    //   - the `CARGO_FEATURE_FFI` env var (so toggling the feature
    //     re-runs the script but nothing else does).
    //
    // The early-return below for non-ffi builds now lets Cargo cache
    // "build.rs produced nothing" against the relevant inputs.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=tests/ffi/lib_testhelper.c");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_FFI");

    // RES-2631: embed build metadata into the compiler binary so
    // `rz --version --verbose` can report the exact build a user is
    // running. This matters for safety-critical embedded engineering
    // where a bug report needs to pin the exact compiler commit.
    //
    // All values fall back to "unknown" when the build environment
    // does not provide them (e.g. a tarball release without `.git`,
    // a sandboxed builder, or a target where `rustc -vV` is not on
    // PATH). The runtime printer must tolerate "unknown" gracefully.
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads");
    emit_build_metadata();

    // Only compile the FFI test helper when the `ffi` feature is active.
    if std::env::var("CARGO_FEATURE_FFI").is_err() {
        return;
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let c_src = PathBuf::from("tests/ffi/lib_testhelper.c");

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

/// RES-2631: emit build-metadata env vars consumed by `--version --verbose`.
///
/// Best-effort: every value falls back to the string `"unknown"` rather
/// than panicking, so a build without `.git` (release tarball, sandbox)
/// still produces a working binary. The runtime is responsible for
/// hiding the "unknown" lines from the verbose printout.
fn emit_build_metadata() {
    let git_hash = run_capture(&["git", "rev-parse", "--short=12", "HEAD"]);
    let git_dirty = run_capture(&["git", "status", "--porcelain"])
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    let git_hash_display = match (git_hash.as_deref(), git_dirty) {
        (Some(h), true) => format!("{}-dirty", h),
        (Some(h), false) => h.to_string(),
        (None, _) => "unknown".to_string(),
    };
    let date = run_capture(&["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .unwrap_or_else(|| "unknown".to_string());
    let target = std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    let profile = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
    let rustc_version = run_capture(&["rustc", "--version"])
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!(
        "cargo:rustc-env=RESILIENT_BUILD_GIT_HASH={}",
        git_hash_display
    );
    println!("cargo:rustc-env=RESILIENT_BUILD_DATE={}", date);
    println!("cargo:rustc-env=RESILIENT_BUILD_TARGET={}", target);
    println!("cargo:rustc-env=RESILIENT_BUILD_PROFILE={}", profile);
    println!(
        "cargo:rustc-env=RESILIENT_BUILD_RUSTC_VERSION={}",
        rustc_version
    );

    // Enabled cargo features show up in env as CARGO_FEATURE_<UPPER>=1.
    // We translate that back to lower-snake names for the user.
    let mut features: Vec<String> = std::env::vars()
        .filter_map(|(k, _)| {
            k.strip_prefix("CARGO_FEATURE_")
                .map(|f| f.to_ascii_lowercase().replace('_', "-"))
        })
        .collect();
    features.sort();
    println!(
        "cargo:rustc-env=RESILIENT_BUILD_FEATURES={}",
        if features.is_empty() {
            "none".to_string()
        } else {
            features.join(",")
        }
    );
}

fn run_capture(argv: &[&str]) -> Option<String> {
    let out = Command::new(argv[0]).args(&argv[1..]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}
