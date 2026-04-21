//! RES-371: optional hook for live-block retry telemetry on `#![no_std]` hosts.
//!
//! The full `resilient` CLI writes NDJSON when `--emit-live-log <file>` is set.
//! Embedded builds that execute live blocks without the driver install a
//! [`LiveTelemetryBackend`] once at startup; [`emit_live_retry`] becomes a
//! no-op when nothing is registered.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

/// Embedder-implemented sink for live-block retry events.
pub trait LiveTelemetryBackend {
    fn on_live_retry(&mut self, block: &str, retry: usize, reason: &str, ts_ns: u64);
}

struct BackendCell(UnsafeCell<Option<*mut (dyn LiveTelemetryBackend + 'static)>>);

unsafe impl Sync for BackendCell {}

static BACKEND: BackendCell = BackendCell(UnsafeCell::new(None));
static BACKEND_PRESENT: AtomicBool = AtomicBool::new(false);

/// True when a backend was installed and not yet cleared.
pub fn has_live_telemetry_backend() -> bool {
    BACKEND_PRESENT.load(Ordering::Relaxed)
}

/// Install `backend` as the global live-telemetry target.
///
/// # Safety
///
/// Same single-threaded-write contract as [`crate::sink::set_sink`](super::sink::set_sink).
pub fn set_live_telemetry(backend: &'static mut dyn LiveTelemetryBackend) {
    BACKEND_PRESENT.store(true, Ordering::Relaxed);
    unsafe {
        *BACKEND.0.get() = Some(backend as *mut dyn LiveTelemetryBackend);
    }
}

/// Remove the installed backend (primarily for tests).
pub fn clear_live_telemetry() {
    BACKEND_PRESENT.store(false, Ordering::Relaxed);
    unsafe {
        *BACKEND.0.get() = None;
    }
}

/// Dispatch one retry event to the installed backend, if any.
pub fn emit_live_retry(block: &str, retry: usize, reason: &str, ts_ns: u64) {
    if !BACKEND_PRESENT.load(Ordering::Relaxed) {
        return;
    }
    unsafe {
        let slot = &mut *BACKEND.0.get();
        let Some(ptr) = slot else {
            return;
        };
        let b: &mut dyn LiveTelemetryBackend = &mut **ptr;
        b.on_live_retry(block, retry, reason, ts_ns);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering as AOrd};

    static LIVE_TELEM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct CountingBackend {
        hits: &'static AtomicUsize,
    }

    impl LiveTelemetryBackend for CountingBackend {
        fn on_live_retry(&mut self, block: &str, retry: usize, reason: &str, ts_ns: u64) {
            self.hits.fetch_add(1, AOrd::Relaxed);
            assert_eq!(block, "demo.rz:3");
            assert_eq!(retry, 1);
            assert!(reason.contains("fail"));
            assert!(ts_ns > 0);
        }
    }

    #[test]
    fn emit_noops_without_backend() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        emit_live_retry("x:1", 1, "r", 1);
        clear_live_telemetry();
    }

    #[test]
    fn emit_delivers_to_installed_backend() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        static HITS: AtomicUsize = AtomicUsize::new(0);
        HITS.store(0, AOrd::Relaxed);
        let backend: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS }));
        set_live_telemetry(backend);
        emit_live_retry("demo.rz:3", 1, "forced fail", 99);
        assert_eq!(HITS.load(AOrd::Relaxed), 1);
        clear_live_telemetry();
    }
}
