// RES-368 / RES-510: WASM-bindgen entry point for the Resilient
// web playground.
//
// Exposes `compile_and_run(source) -> JSON` so the browser can drive
// the language without going through a server. The playground is a
// "demo, not a full toolchain" surface — JIT, FFI, Z3, file I/O, and
// the watcher are intentionally absent (none compile to WASM today,
// and the playground value is the language semantics, not the
// supporting tooling).
//
// RES-510 PR 3: previously a stub that echoed the source text. Now
// calls into the real `resilient::run_program` interpreter — the lib
// refactor (PR 1) and injectable stdout sink (PR 2) made that
// possible. The CLI-only deps in `resilient` are cfg-gated on
// `not(target_arch = "wasm32")`, so this crate compiles to
// `wasm32-unknown-unknown` without dragging termios or fs-watcher
// platform APIs in.

#![allow(clippy::needless_pass_by_value)]

use serde::Serialize;
use wasm_bindgen::prelude::*;

/// Result of running a source snippet. The browser-side JS turns this
/// into the editor's output pane. Kept JSON-stable so the schema can
/// extend (diagnostics, timing) without breaking the page.
#[derive(Serialize)]
struct RunResult {
    /// Concatenated stdout from the run. May be empty on success
    /// when the program produces no output.
    stdout: String,
    /// Concatenated stderr / diagnostics from the run. `None` when
    /// the run succeeded; `Some(text)` carries the diagnostic block
    /// the user should see.
    stderr: Option<String>,
    /// Process-style exit status. `0` for success, non-zero for any
    /// diagnostic-producing failure.
    exit_code: i32,
    /// Wall-clock milliseconds the run took, for the result pane's
    /// "ran in N ms" footer.
    duration_ms: f64,
    /// Build flavor — `"tree-walker"` once the real interpreter is
    /// wired (RES-510 PR 3). The page's "scaffold" banner hides
    /// itself on any non-`"stub"` flavor.
    flavor: &'static str,
}

/// Run a Resilient source snippet and return a JSON `RunResult`.
///
/// `source` is the editor buffer; `_input` is reserved for future
/// stdin-style input (e.g. when the playground grows examples that
/// read user keystrokes — `input()` calls today block on real
/// stdin and won't return useful data inside the WASM sandbox).
#[wasm_bindgen]
pub fn compile_and_run(source: &str, _input: &str) -> JsValue {
    let started = now_ms();
    let result = run_inner(source);
    let elapsed = (now_ms() - started).max(0.0);
    let payload = RunResult {
        stdout: result.stdout,
        stderr: result.stderr,
        exit_code: result.exit_code,
        duration_ms: elapsed,
        flavor: "tree-walker",
    };
    serde_wasm_bindgen::to_value(&payload).unwrap_or(JsValue::NULL)
}

/// `compile_and_run` implementation, factored out so it can be
/// exercised from native `cargo test` without touching `JsValue`.
///
/// Calls `resilient::run_program` (RES-510 PR 2) which captures
/// stdout into a buffer and returns parser / runtime errors as a
/// flat list of human-readable lines. The mapping into the
/// `InnerResult` shape is intentionally narrow:
///
/// * `ok == true`  → `exit_code = 0`, stderr `None`.
/// * `ok == false` → `exit_code = 1`, errors joined with newlines
///   into stderr. Any partial stdout (lines printed before a
///   runtime error) flows through unchanged.
fn run_inner(source: &str) -> InnerResult {
    if source.trim().is_empty() {
        return InnerResult {
            stdout: String::new(),
            stderr: Some("error: empty program".to_owned()),
            exit_code: 2,
        };
    }

    let result = resilient::run_program(source);
    if result.ok {
        InnerResult {
            stdout: result.stdout,
            stderr: None,
            exit_code: 0,
        }
    } else {
        InnerResult {
            stdout: result.stdout,
            stderr: Some(result.errors.join("\n")),
            exit_code: 1,
        }
    }
}

struct InnerResult {
    stdout: String,
    stderr: Option<String>,
    exit_code: i32,
}

/// Library version string (e.g. `"0.1.0-tree-walker"`) so the page
/// banner can pin a build to a known version when filing bugs.
#[wasm_bindgen]
pub fn playground_version() -> String {
    format!("{}-tree-walker", env!("CARGO_PKG_VERSION"))
}

/// Best-effort wall-clock millisecond timer. Uses
/// `js_sys::Date::now()` in the browser and `std::time` natively so
/// the same code path is exercised by `cargo test`.
fn now_ms() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_is_diagnosed() {
        let r = run_inner("");
        assert_eq!(r.exit_code, 2);
        assert!(r.stderr.is_some());
    }

    #[test]
    fn whitespace_only_is_also_empty() {
        let r = run_inner("   \n\t\n");
        assert_eq!(r.exit_code, 2);
    }

    #[test]
    fn hello_world_runs_through_real_interpreter() {
        let r = run_inner(r#"println("Hello, Resilient world!");"#);
        assert_eq!(r.exit_code, 0, "stderr: {:?}", r.stderr);
        assert!(r.stderr.is_none());
        assert_eq!(r.stdout, "Hello, Resilient world!\n");
    }

    #[test]
    fn arithmetic_evaluates_and_prints() {
        let r = run_inner(
            r#"
            let x = 40;
            let y = x + 2;
            println(y);
            "#,
        );
        assert_eq!(r.exit_code, 0, "stderr: {:?}", r.stderr);
        assert_eq!(r.stdout, "42\n");
    }

    #[test]
    fn parser_error_surfaces_in_stderr() {
        let r = run_inner("let x = ;");
        assert_eq!(r.exit_code, 1);
        let msg = r.stderr.expect("expected stderr");
        assert!(!msg.is_empty(), "stderr was empty");
        assert!(r.stdout.is_empty());
    }

    #[test]
    fn runtime_error_keeps_partial_stdout() {
        let r = run_inner(
            r#"
            println("before");
            let x = 1 / 0;
            println("after");
            "#,
        );
        assert_eq!(r.exit_code, 1);
        assert!(r.stdout.contains("before"), "stdout: {:?}", r.stdout);
        assert!(!r.stdout.contains("after"));
        assert!(r.stderr.is_some());
    }

    #[test]
    fn version_string_is_no_longer_stub() {
        let v = playground_version();
        assert!(v.ends_with("-tree-walker"), "version: {}", v);
    }
}
