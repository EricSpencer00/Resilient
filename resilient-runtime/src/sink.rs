//! RES-180: output-sink abstraction for `println` / `print`.
//!
//! The runtime has no stdout concept on embedded — users wire
//! a UART, semihosting, or ring-buffer backend, and each
//! project picks one. This module lets the runtime emit
//! text through a user-installed trait object instead of
//! a hard-coded sink.
//!
//! # Shape
//!
//! - `Sink` — one-method trait (`write_str`) the user implements.
//! - `SinkErr` — the trait's error type.
//! - `set_sink(sink)` — install a `&'static mut dyn Sink` as the
//!   global output target. Call once at program start; later
//!   calls overwrite.
//! - `print(s)` / `println(s)` — write through the currently-
//!   installed sink. Returns `Err(SinkErr::NoSink)` when none
//!   is installed.
//!
//! Under `--features std-sink`, a `StdoutSink` convenience type
//! is available that forwards to `std::io::stdout()`. Users who
//! want the old std-host behavior wire it once at startup:
//!
//! ```ignore
//! use resilient_runtime::sink::{set_sink, StdoutSink};
//! static mut STDOUT: StdoutSink = StdoutSink;
//! unsafe { set_sink(&mut STDOUT); }
//! ```
//!
//! # Thread-safety
//!
//! The global sink pointer lives in an `UnsafeCell` behind a
//! `Sync` newtype. This is sound for embedded bare-metal
//! (single-core / single-thread is the overwhelmingly common
//! case) and for the runtime's unit tests (which serialize
//! sink access via `SINK_TEST_LOCK` — see the `tests` submodule).
//! A future ticket that introduces actual multi-threaded
//! embedded use will need to either gate this cell behind
//! `critical-section` or wrap it in a `spin::Mutex`.

use core::cell::UnsafeCell;

/// Error returned by `Sink::write_str` and the module-level
/// `print` / `println` helpers. The variants are exhaustive and
/// deliberately narrow — new variants will land as new
/// functionality requires them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkErr {
    /// No sink has been installed via `set_sink` yet. The caller
    /// either forgot to wire one at program start, or is calling
    /// the runtime's `println` from a context where the sink
    /// hasn't been set up.
    NoSink,
    /// The installed sink's `write_str` returned an error. The
    /// payload is deliberately unit — sinks encode their own
    /// richer diagnostics in their implementation and surface
    /// them via logging / side channels.
    WriteFailed,
}

/// User-implementable output trait. Bare-metal users wire this
/// to a UART / semihosting write; ring-buffer backends push the
/// bytes into a circular queue; test code captures into a Vec<u8>.
///
/// Implementations MUST be non-blocking in realtime contexts —
/// `println` is called from user code that may run with
/// interrupts masked. (The runtime itself doesn't enforce this
/// — it's the sink writer's responsibility.)
pub trait Sink {
    fn write_str(&mut self, s: &str) -> Result<(), SinkErr>;
}

/// RES-180: global holder for the currently-installed sink. The
/// `Sync` impl below is the sound-only-for-single-threaded
/// promise documented in the module header.
struct SinkCell(UnsafeCell<Option<*mut (dyn Sink + 'static)>>);

// SAFETY: valid only under the single-threaded-or-serialized
// access invariant documented at module level. Embedded bare-
// metal: single-core. Runtime tests: SINK_TEST_LOCK serializes.
// Multi-threaded embedded: not supported yet; see module docs.
unsafe impl Sync for SinkCell {}

static OUT: SinkCell = SinkCell(UnsafeCell::new(None));

/// Install `sink` as the global output sink. Overwrites any
/// previous installation. After this call, `print` / `println`
/// route text through `sink.write_str`.
///
/// # Lifetime
///
/// `&'static mut dyn Sink` means the sink outlives the program
/// — the common bare-metal pattern is `static mut MY_SINK: MyUart
/// = ...; set_sink(&mut MY_SINK)` at `#[entry]` time.
///
/// # Safety
///
/// Safe in single-threaded contexts (all embedded bare-metal,
/// and serialized test use). Not safe to call from multiple
/// threads concurrently; synchronize externally if your
/// deployment is threaded.
pub fn set_sink(sink: &'static mut dyn Sink) {
    // SAFETY: single-threaded-write invariant — see module docs.
    unsafe {
        *OUT.0.get() = Some(sink as *mut dyn Sink);
    }
}

/// Clear the currently-installed sink. Primarily for tests so
/// they can assert the "no sink" error path cleanly; production
/// code almost never needs this.
pub fn clear_sink() {
    // SAFETY: same invariant as `set_sink`.
    unsafe {
        *OUT.0.get() = None;
    }
}

/// Write `s` through the current sink. Returns `Err(NoSink)` if
/// no sink has been installed. This is the primitive both
/// `print` and `println` compose on top of.
pub fn print(s: &str) -> Result<(), SinkErr> {
    // SAFETY: single-threaded-access invariant. The `*mut dyn
    // Sink` we hold came from a `&'static mut` passed into
    // `set_sink`, so it's a valid mutable reference to a
    // live sink (lifetime `'static`).
    unsafe {
        let slot = &mut *OUT.0.get();
        match slot {
            Some(ptr) => {
                let sink: &mut dyn Sink = &mut **ptr;
                sink.write_str(s)
            }
            None => Err(SinkErr::NoSink),
        }
    }
}

/// `print(s)` followed by `print("\n")`. Atomicity across the
/// two writes is NOT guaranteed — if a sink can be preempted
/// mid-write, interleaving is possible. Protect via the sink
/// implementation if that matters (e.g. wrap each `write_str`
/// in a critical section).
pub fn println(s: &str) -> Result<(), SinkErr> {
    print(s)?;
    print("\n")
}

// ---------- optional std-sink convenience ----------

/// `std::io::stdout()`-forwarding sink. Only compiled with the
/// `std-sink` feature — pulls in `std`, so it's unavailable in
/// no_std builds. Install once at program start with
/// `set_sink(&mut STDOUT_SINK)`.
#[cfg(feature = "std-sink")]
pub struct StdoutSink;

#[cfg(feature = "std-sink")]
impl Sink for StdoutSink {
    fn write_str(&mut self, s: &str) -> Result<(), SinkErr> {
        use std::io::Write;
        std::io::stdout()
            .write_all(s.as_bytes())
            .map_err(|_| SinkErr::WriteFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Serialize all sink-touching tests so they don't race on
    /// the global `OUT` cell. Same shape as the RES-150 RNG
    /// lock pattern.
    static SINK_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Memory-backed sink — captures every write into a Vec<u8>
    /// so tests can assert on the emitted bytes.
    struct BufSink {
        buf: Vec<u8>,
    }

    impl Sink for BufSink {
        fn write_str(&mut self, s: &str) -> Result<(), SinkErr> {
            self.buf.extend_from_slice(s.as_bytes());
            Ok(())
        }
    }

    /// A BufSink variant that ALWAYS errors — exercises the
    /// error propagation path through `print` / `println`.
    struct FailSink;
    impl Sink for FailSink {
        fn write_str(&mut self, _s: &str) -> Result<(), SinkErr> {
            Err(SinkErr::WriteFailed)
        }
    }

    /// Leak a heap-allocated BufSink to get a `&'static mut` —
    /// tests never run long enough for the leak to matter, and
    /// the `'static` lifetime is what `set_sink` needs.
    fn leak_buf_sink() -> &'static mut BufSink {
        alloc::boxed::Box::leak(alloc::boxed::Box::new(BufSink { buf: Vec::new() }))
    }

    #[test]
    fn print_writes_to_installed_sink() {
        let _g = SINK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sink = leak_buf_sink();
        let sink_ptr: *mut BufSink = sink as *mut _;
        set_sink(sink);
        print("hello").unwrap();
        print(", world").unwrap();
        clear_sink();
        // Reach back through the raw ptr to inspect buf. Safe
        // because `set_sink` dropped out of the global slot
        // (clear_sink) and no one else is holding a reference.
        let observed: String = unsafe {
            let s = &*sink_ptr;
            String::from_utf8(s.buf.clone()).unwrap()
        };
        assert_eq!(observed, "hello, world");
    }

    #[test]
    fn println_appends_newline() {
        let _g = SINK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sink = leak_buf_sink();
        let sink_ptr: *mut BufSink = sink as *mut _;
        set_sink(sink);
        println("line-a").unwrap();
        println("line-b").unwrap();
        clear_sink();
        let observed: String = unsafe {
            let s = &*sink_ptr;
            String::from_utf8(s.buf.clone()).unwrap()
        };
        assert_eq!(observed, "line-a\nline-b\n");
    }

    #[test]
    fn print_without_sink_returns_nosink_error() {
        let _g = SINK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_sink();
        assert_eq!(print("x").unwrap_err(), SinkErr::NoSink);
        assert_eq!(println("y").unwrap_err(), SinkErr::NoSink);
    }

    #[test]
    fn failing_sink_surfaces_write_failed() {
        let _g = SINK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sink: &'static mut FailSink = alloc::boxed::Box::leak(alloc::boxed::Box::new(FailSink));
        set_sink(sink);
        let err = print("anything").unwrap_err();
        clear_sink();
        assert_eq!(err, SinkErr::WriteFailed);
    }

    #[test]
    fn set_sink_replaces_previous_installation() {
        let _g = SINK_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let first = leak_buf_sink();
        let first_ptr: *mut BufSink = first as *mut _;
        let second = leak_buf_sink();
        let second_ptr: *mut BufSink = second as *mut _;

        set_sink(first);
        print("to-first").unwrap();

        set_sink(second); // replaces
        print("to-second").unwrap();

        clear_sink();
        let first_seen = unsafe { String::from_utf8((*first_ptr).buf.clone()).unwrap() };
        let second_seen = unsafe { String::from_utf8((*second_ptr).buf.clone()).unwrap() };
        assert_eq!(first_seen, "to-first");
        assert_eq!(second_seen, "to-second");
    }

    // Feature-gated StdoutSink check: can be constructed and
    // its write_str compiles when --features std-sink is on.
    // We don't run it (would actually print to stdout during
    // `cargo test`), just make sure the type is valid.
    #[cfg(feature = "std-sink")]
    #[test]
    fn stdout_sink_is_constructible() {
        let mut s = StdoutSink;
        // Smoke: write an empty string, should succeed without
        // producing visible test noise.
        assert!(s.write_str("").is_ok());
    }
}
