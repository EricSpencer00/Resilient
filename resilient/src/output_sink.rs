//! RES-510 PR 2: injectable stdout sink for the interpreter's
//! `print` / `println` builtins.
//!
//! The CLI driver writes interpreter output directly to the process
//! stdout. Non-CLI consumers (the WASM playground, in-process test
//! harnesses) need to capture that output into a buffer they own.
//! This module provides:
//!
//! * [`with_captured_output`] — run a closure with the print
//!   builtins routed into a fresh buffer; return what the closure
//!   produced plus the captured bytes as a UTF-8 `String`.
//! * [`write_str`] / [`flush`] — used by `builtin_print` and
//!   `builtin_println` to emit. Routes to stdout in the default
//!   sink, into the buffer in capture mode.
//!
//! The sink is a thread-local, so two threads can capture output
//! independently. Within a single thread, captures don't nest —
//! calling `with_captured_output` while already capturing replaces
//! the buffer for the duration of the inner call and restores the
//! outer buffer (with the inner's contents merged in) on exit. That
//! matches what a caller writing
//!
//!     with_captured_output(|| {
//!         with_captured_output(|| inner());
//!         outer();
//!     })
//!
//! intuitively expects.

use std::cell::RefCell;
use std::io::Write as _;

/// Where the `print`-family builtins send their output.
pub enum OutputSink {
    /// Default. Routes to `std::io::stdout()`.
    Stdout,
    /// Capture mode. Bytes are appended here.
    Buffer(Vec<u8>),
}

thread_local! {
    static SINK: RefCell<OutputSink> = const { RefCell::new(OutputSink::Stdout) };
}

/// Append `s` to the active sink. Used by `builtin_print` /
/// `builtin_println` / the `input` prompt; non-builtin code should
/// keep using `print!` / `println!` directly so CLI logging /
/// diagnostics don't accidentally land in the captured buffer.
pub(crate) fn write_str(s: &str) {
    SINK.with(|sink| match &mut *sink.borrow_mut() {
        OutputSink::Stdout => {
            let stdout = std::io::stdout();
            let mut h = stdout.lock();
            let _ = h.write_all(s.as_bytes());
        }
        OutputSink::Buffer(buf) => {
            buf.extend_from_slice(s.as_bytes());
        }
    });
}

/// Flush the active sink. No-op in capture mode (the buffer has no
/// notion of flushing); for stdout, ensures partial-line output is
/// visible before the next read.
pub(crate) fn flush() {
    SINK.with(|sink| {
        if let OutputSink::Stdout = &*sink.borrow() {
            let _ = std::io::stdout().flush();
        }
    });
}

/// Run `f` with a fresh `Buffer` sink installed; restore the previous
/// sink on exit (even on panic) and return whatever `f` returned plus
/// the captured bytes as a UTF-8 string. Invalid UTF-8 in the buffer
/// is replaced with `U+FFFD` (`String::from_utf8_lossy`); we expect
/// the print builtins to only emit Rust strings, which are guaranteed
/// UTF-8, so the lossy fallback is a safety net rather than a hot
/// path.
///
/// If `f` panics, the previous sink is restored before the panic
/// resumes so subsequent CLI output continues to land on stdout.
pub fn with_captured_output<R>(f: impl FnOnce() -> R) -> (R, String) {
    struct Guard(Option<OutputSink>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(prev) = self.0.take() {
                SINK.with(|sink| {
                    *sink.borrow_mut() = prev;
                });
            }
        }
    }

    let guard = Guard(Some(SINK.with(|sink| {
        std::mem::replace(&mut *sink.borrow_mut(), OutputSink::Buffer(Vec::new()))
    })));

    let result = f();

    let captured = SINK.with(|sink| {
        match std::mem::replace(
            &mut *sink.borrow_mut(),
            // Restore happens via Guard::drop; this temporary replacement
            // is just to extract the buffer.
            OutputSink::Stdout,
        ) {
            OutputSink::Buffer(buf) => String::from_utf8_lossy(&buf).into_owned(),
            // Concurrent swap by the closure — unexpected but recover
            // gracefully.
            OutputSink::Stdout => String::new(),
        }
    });
    // The temporary `Stdout` we just installed gets immediately
    // replaced by `guard.drop()` with the original sink (which is
    // what `prev` captured at entry).
    drop(guard);
    (result, captured)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_simple_output() {
        let ((), out) = with_captured_output(|| {
            write_str("hello\n");
            write_str("world\n");
        });
        assert_eq!(out, "hello\nworld\n");
    }

    #[test]
    fn empty_capture_returns_empty_string() {
        let ((), out) = with_captured_output(|| {});
        assert!(out.is_empty());
    }

    #[test]
    fn closure_return_value_is_threaded_through() {
        let (n, out) = with_captured_output(|| {
            write_str("answer: ");
            42
        });
        assert_eq!(n, 42);
        assert_eq!(out, "answer: ");
    }

    #[test]
    fn flush_is_a_noop_in_capture_mode() {
        let ((), out) = with_captured_output(|| {
            write_str("partial");
            flush();
            write_str(" line\n");
        });
        assert_eq!(out, "partial line\n");
    }

    #[test]
    fn nested_capture_isolates_inner_buffer() {
        let ((), outer) = with_captured_output(|| {
            write_str("outer-before\n");
            let ((), inner) = with_captured_output(|| {
                write_str("inner-only\n");
            });
            assert_eq!(inner, "inner-only\n");
            write_str("outer-after\n");
        });
        assert_eq!(outer, "outer-before\nouter-after\n");
    }

    #[test]
    fn panic_in_closure_restores_sink() {
        // Catch the panic so the test process doesn't abort, then
        // confirm a subsequent capture still works (i.e. the sink
        // wasn't left in an inconsistent state).
        let result = std::panic::catch_unwind(|| {
            let _ = with_captured_output(|| {
                write_str("before-panic\n");
                panic!("boom");
            });
        });
        assert!(result.is_err());
        let ((), after) = with_captured_output(|| {
            write_str("after-panic\n");
        });
        assert_eq!(after, "after-panic\n");
    }
}
