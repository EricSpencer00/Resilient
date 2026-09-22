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
static BACKEND_LOCK: AtomicBool = AtomicBool::new(false);

struct BackendGuard;

impl BackendGuard {
    #[cfg(target_has_atomic = "8")]
    fn try_lock() -> Option<Self> {
        BACKEND_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| Self)
    }

    #[cfg(not(target_has_atomic = "8"))]
    fn try_lock() -> Option<Self> {
        Some(Self)
    }
}

impl Drop for BackendGuard {
    fn drop(&mut self) {
        BACKEND_LOCK.store(false, Ordering::Release);
    }
}

/// True when a backend was installed and not yet cleared.
pub fn has_live_telemetry_backend() -> bool {
    BACKEND_PRESENT.load(Ordering::Acquire)
}

/// Install `backend` as the global live-telemetry target. If another
/// telemetry operation is active, the installation is ignored.
pub fn set_live_telemetry(backend: &'static mut dyn LiveTelemetryBackend) {
    let Some(_guard) = BackendGuard::try_lock() else {
        return;
    };
    unsafe {
        *BACKEND.0.get() = Some(backend as *mut dyn LiveTelemetryBackend);
    }
    BACKEND_PRESENT.store(true, Ordering::Release);
}

/// Remove the installed backend (primarily for tests). If another
/// telemetry operation is active, the removal is ignored.
pub fn clear_live_telemetry() {
    let Some(_guard) = BackendGuard::try_lock() else {
        return;
    };
    unsafe {
        *BACKEND.0.get() = None;
    }
    BACKEND_PRESENT.store(false, Ordering::Release);
}

/// Dispatch one retry event to the installed backend, if any. A reentrant
/// or concurrent emission is dropped instead of aliasing the mutable backend.
pub fn emit_live_retry(block: &str, retry: usize, reason: &str, ts_ns: u64) {
    let Some(_guard) = BackendGuard::try_lock() else {
        return;
    };
    if !BACKEND_PRESENT.load(Ordering::Acquire) {
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

    // ---------- Hook swap sequencing ----------

    #[test]
    fn has_backend_starts_false() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        assert!(!has_live_telemetry_backend());
    }

    #[test]
    fn has_backend_true_after_set() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let backend: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS }));
        set_live_telemetry(backend);
        assert!(has_live_telemetry_backend());
        clear_live_telemetry();
    }

    #[test]
    fn has_backend_false_after_clear() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        static HITS: AtomicUsize = AtomicUsize::new(0);
        let backend: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS }));
        set_live_telemetry(backend);
        assert!(has_live_telemetry_backend());
        clear_live_telemetry();
        assert!(!has_live_telemetry_backend());
    }

    #[test]
    fn backend_swap_replaces_target() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        static HITS1: AtomicUsize = AtomicUsize::new(0);
        static HITS2: AtomicUsize = AtomicUsize::new(0);
        HITS1.store(0, AOrd::Relaxed);
        HITS2.store(0, AOrd::Relaxed);
        let backend1: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS1 }));
        let backend2: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS2 }));
        // Install first backend
        set_live_telemetry(backend1);
        emit_live_retry("demo.rz:3", 1, "fail", 99);
        assert_eq!(HITS1.load(AOrd::Relaxed), 1);
        // Replace with second backend
        set_live_telemetry(backend2);
        emit_live_retry("demo.rz:3", 1, "fail", 99);
        // First backend saw 1, second backend saw 1.
        assert_eq!(HITS1.load(AOrd::Relaxed), 1);
        assert_eq!(HITS2.load(AOrd::Relaxed), 1);
        clear_live_telemetry();
    }

    #[test]
    fn multiple_emit_calls_accumulate() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_live_telemetry();
        static HITS: AtomicUsize = AtomicUsize::new(0);
        HITS.store(0, AOrd::Relaxed);
        let backend: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS }));
        set_live_telemetry(backend);
        for i in 0..5 {
            emit_live_retry("demo.rz:3", 1, "fail", 99 + i);
        }
        assert_eq!(HITS.load(AOrd::Relaxed), 5);
        clear_live_telemetry();
    }

    #[test]
    fn clear_then_emit_is_noop() {
        let _g = LIVE_TELEM_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        static HITS: AtomicUsize = AtomicUsize::new(0);
        HITS.store(0, AOrd::Relaxed);
        let backend: &'static mut CountingBackend =
            Box::leak(Box::new(CountingBackend { hits: &HITS }));
        set_live_telemetry(backend);
        clear_live_telemetry();
        emit_live_retry("demo.rz:3", 1, "fail", 99);
        assert_eq!(HITS.load(AOrd::Relaxed), 0);
    }
}
