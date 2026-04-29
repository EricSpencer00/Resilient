// RES-368: WASM-bindgen entry point for the Resilient web playground.
//
// Exposes `compile_and_run(source) -> JSON` so the browser can drive
// the language without going through a server. The playground is a
// "demo, not a full toolchain" surface — JIT, FFI, Z3, file I/O, and
// the watcher are intentionally absent (none compile to WASM today,
// and the playground value is the language semantics, not the
// supporting tooling).
//
// Status: scaffold. The interpreter integration is deferred until
// `resilient/Cargo.toml` exposes a library target — currently it is
// `[[bin]]`-only (see comment at `resilient/Cargo.toml:138`). Tracked
// in the PR description for #160.

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
    /// Build flavor — `"stub"` until the interpreter integration
    /// lands, then `"tree-walker"`. Surfaces to the UI so the
    /// "scaffold" banner can hide once the real interpreter is wired.
    flavor: &'static str,
}

/// Run a Resilient source snippet and return a JSON `RunResult`.
///
/// `source` is the editor buffer; `_input` is reserved for future
/// stdin-style input (e.g. when the playground grows examples that
/// read user keystrokes).
///
/// Today this is a stub that echoes the program text and surfaces a
/// "scaffold" notice, so the page round-trip (editor → wasm → output)
/// is verifiable end-to-end before the interpreter integration lands.
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
        flavor: "stub",
    };
    serde_wasm_bindgen::to_value(&payload).unwrap_or(JsValue::NULL)
}

/// `compile_and_run` implementation, factored out so it can be
/// exercised from native `cargo test` without touching `JsValue`.
fn run_inner(source: &str) -> InnerResult {
    if source.trim().is_empty() {
        return InnerResult {
            stdout: String::new(),
            stderr: Some("error: empty program".to_owned()),
            exit_code: 2,
        };
    }

    let preview = source.lines().take(5).collect::<Vec<_>>().join("\n");
    let stdout = format!(
        "[playground scaffold — interpreter integration pending]\n\
         received {} bytes; first {} line(s):\n\
         {}\n",
        source.len(),
        source.lines().take(5).count(),
        preview
    );
    InnerResult {
        stdout,
        stderr: None,
        exit_code: 0,
    }
}

struct InnerResult {
    stdout: String,
    stderr: Option<String>,
    exit_code: i32,
}

/// Library version string (e.g. `"0.1.0-stub"`) so the page banner
/// can pin a build to a known scaffold version when filing bugs.
#[wasm_bindgen]
pub fn playground_version() -> String {
    format!("{}-stub", env!("CARGO_PKG_VERSION"))
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
    fn nonempty_source_is_echoed_with_scaffold_marker() {
        let r = run_inner("fn main() { println(\"hi\"); }");
        assert_eq!(r.exit_code, 0);
        assert!(r.stderr.is_none());
        assert!(r.stdout.contains("playground scaffold"));
        assert!(r.stdout.contains("first 1 line(s)"));
    }
}
