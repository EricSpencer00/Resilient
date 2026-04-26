//! RES-374: heap profiler — peak allocation tracking.
//!
//! A thin instrumentation shim for the `#[global_allocator]` so
//! firmware authors can observe peak heap usage without giving up
//! their existing allocator. The shim is `#[cfg(feature = "alloc")]`
//! gated; on no-alloc builds the public API still compiles but
//! reports zero so callers stay portable across feature sets.
//!
//! # Why a wrapper?
//!
//! The runtime crate doesn't install a `#[global_allocator]` — that
//! is the binary's responsibility (see
//! `resilient-runtime-cortex-m-demo` for the canonical wiring). The
//! profiler therefore can't intercept a third-party allocator from
//! the outside; it has to be in the alloc path. The
//! [`TrackingHeap`] type wraps any [`core::alloc::GlobalAlloc`]
//! implementor and forwards every call, updating two atomics
//! (current bytes, peak bytes) before returning. The user's
//! `#[global_allocator]` becomes
//! `TrackingHeap<embedded_alloc::Heap>` instead of
//! `embedded_alloc::Heap`.
//!
//! # Why globals (not per-instance state)?
//!
//! `peak_bytes()` and `reset_peak()` need to be callable from
//! anywhere in the user's program without threading an allocator
//! handle through every function. There is exactly one global
//! allocator, so a static `AtomicUsize` for the peak high-water
//! mark and a static `AtomicUsize` for the current usage are the
//! natural choice. `Ordering::Relaxed` is sufficient — we are
//! reading approximate counters, not synchronising against another
//! data structure.
//!
//! # Sizing assumption
//!
//! The counters are `AtomicUsize`. On 16-bit MCUs (`avr`, MSP430)
//! that's 16 bits, so a heap larger than 64 KiB would saturate the
//! counter. Embedded targets currently in scope (Cortex-M /
//! RISC-V) are at least 32-bit so this is comfortably wide. The
//! counters never wrap on dealloc — `wrapping_sub` would silently
//! mask a bug; instead we `saturating_sub` so a counter mismatch
//! shows up as a stuck-at-zero rather than a corrupted huge
//! number.

#[cfg(feature = "alloc")]
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Bytes currently held by live allocations through the tracker.
/// Updated by every `alloc` (+= size) and `dealloc` (-= size).
static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Peak `CURRENT_BYTES` observed since program start (or last
/// [`reset_peak`] call).
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Peak heap usage in bytes, measured across all allocations made
/// through a [`TrackingHeap`] global allocator since program start
/// or the last [`reset_peak`] call.
///
/// Returns `0` on builds without the `alloc` feature — the runtime
/// has no heap to track in that posture. This lets portable code
/// call `peak_bytes()` unconditionally.
///
/// # Example
///
/// ```ignore
/// let before = resilient_runtime::heap::peak_bytes();
/// let v = alloc::vec![0u8; 1024];
/// let after = resilient_runtime::heap::peak_bytes();
/// assert!(after >= before + 1024);
/// drop(v);
/// ```
#[inline]
pub fn peak_bytes() -> usize {
    PEAK_BYTES.load(Ordering::Relaxed)
}

/// Resets the peak high-water mark to the current live-allocation
/// total. Useful between phases of a program when you want to
/// measure peak usage of a single phase rather than over the
/// program lifetime.
///
/// On builds without the `alloc` feature this is a no-op; callers
/// can invoke it unconditionally.
#[inline]
pub fn reset_peak() {
    let now = CURRENT_BYTES.load(Ordering::Relaxed);
    PEAK_BYTES.store(now, Ordering::Relaxed);
}

/// Bytes currently held by live allocations. Useful as a
/// diagnostic in tests; firmware should usually consult
/// [`peak_bytes`] for budgeting.
///
/// Returns `0` on builds without the `alloc` feature.
#[inline]
pub fn current_bytes() -> usize {
    CURRENT_BYTES.load(Ordering::Relaxed)
}

/// A `#[global_allocator]`-compatible wrapper that forwards every
/// allocation to an inner [`GlobalAlloc`] and records the byte
/// totals in [`current_bytes`] / [`peak_bytes`] counters.
///
/// Construct it `const`-ly so it can live in a `static`:
///
/// ```ignore
/// use resilient_runtime::heap::TrackingHeap;
/// use embedded_alloc::Heap;
///
/// #[global_allocator]
/// static HEAP: TrackingHeap<Heap> = TrackingHeap::new(Heap::empty());
/// ```
///
/// # Soundness
///
/// `unsafe impl GlobalAlloc` here is sound because:
/// - Every method delegates immediately to the inner allocator;
///   the wrapper does no memory layout work of its own.
/// - The atomics are updated only AFTER a successful `alloc` (or
///   BEFORE a `dealloc`, since the layout's size doesn't change
///   between alloc/dealloc). A counter race never produces an
///   invalid pointer — at worst it under- or over-reports a few
///   bytes briefly.
/// - The wrapper holds the inner allocator by value; lifetime is
///   identical to a non-wrapped `static A` allocator.
#[cfg(feature = "alloc")]
pub struct TrackingHeap<A: GlobalAlloc> {
    inner: A,
}

#[cfg(feature = "alloc")]
impl<A: GlobalAlloc> TrackingHeap<A> {
    /// Wrap an inner allocator. `const` so it can be assigned to a
    /// `static` global allocator slot.
    pub const fn new(inner: A) -> Self {
        Self { inner }
    }

    /// Borrow the inner allocator — useful if it has a setup
    /// method that needs to be called once at boot (e.g.
    /// `embedded_alloc::Heap::init`).
    pub fn inner(&self) -> &A {
        &self.inner
    }
}

// SAFETY: see the soundness note on `TrackingHeap`. We forward all
// methods to `self.inner` and only update relaxed atomics around
// the call.
#[cfg(feature = "alloc")]
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingHeap<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding the caller's `Layout` unchanged to the
        // inner allocator preserves the GlobalAlloc contract.
        let ptr = unsafe { self.inner.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record_dealloc(layout.size());
        // SAFETY: the caller upholds GlobalAlloc::dealloc's
        // preconditions; we forward `ptr` and `layout` unchanged.
        unsafe { self.inner.dealloc(ptr, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarding unchanged. `alloc_zeroed`'s contract
        // is the same as `alloc`'s plus a zero-init guarantee, and
        // we don't observe the returned memory's contents.
        let ptr = unsafe { self.inner.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // realloc may move the allocation. The byte delta is
        // (new_size - old_size); we capture old_size BEFORE the
        // call so `layout.size()` still describes the current
        // allocation, and adjust on success.
        let old_size = layout.size();
        // SAFETY: forwarding unchanged.
        let new_ptr = unsafe { self.inner.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            // realloc semantics: the old allocation is freed
            // (whether or not the pointer moved), the new one is
            // live. Net delta = new_size - old_size.
            if new_size >= old_size {
                record_alloc(new_size - old_size);
            } else {
                record_dealloc(old_size - new_size);
            }
        }
        new_ptr
    }
}

/// Bump CURRENT and bubble PEAK if needed. Always-built so
/// `TrackingHeap` and any future allocator-side instrumentation can
/// share the helper.
#[cfg(feature = "alloc")]
fn record_alloc(size: usize) {
    let new_current = CURRENT_BYTES.fetch_add(size, Ordering::Relaxed) + size;
    // CAS-loop bubble: if PEAK < new_current, update it. We don't
    // care which contender wins — they're racing on a monotonically
    // non-decreasing watermark.
    let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
    while peak < new_current {
        match PEAK_BYTES.compare_exchange_weak(
            peak,
            new_current,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => peak = observed,
        }
    }
}

/// Saturating-sub so a counter mismatch surfaces as a stuck-at-zero
/// rather than a wraparound to ~usize::MAX.
#[cfg(feature = "alloc")]
fn record_dealloc(size: usize) {
    // `fetch_sub` would wrap; do a CAS so we saturate at 0.
    let mut current = CURRENT_BYTES.load(Ordering::Relaxed);
    loop {
        let new_val = current.saturating_sub(size);
        match CURRENT_BYTES.compare_exchange_weak(
            current,
            new_val,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(observed) => current = observed,
        }
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use core::alloc::Layout;
    extern crate alloc as alloc_crate;

    /// A trivially-sound test allocator — `alloc::alloc::Global` is
    /// not stable, so we forward to the system allocator via
    /// `std::alloc::System`. The host test harness compiles with
    /// `std`, so this is fine; no_std users wire their own.
    struct SystemAlloc;
    // SAFETY: forwarding unchanged to the system allocator.
    unsafe impl GlobalAlloc for SystemAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            unsafe { std::alloc::System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { std::alloc::System.dealloc(ptr, layout) }
        }
    }

    /// Exercising the tracker in a unit test requires a fresh
    /// counter each test. The static counters are global, so we
    /// reset them at the start of each test that asserts on them
    /// and use an exclusive lock to serialise (the harness runs
    /// tests in parallel by default, which would race on the
    /// globals).
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        // The tests in this module are short and panic-free; if
        // one panics, subsequent acquirers will see a poisoned
        // lock — recover by taking the inner guard.
        match M.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn reset_all() {
        CURRENT_BYTES.store(0, Ordering::Relaxed);
        PEAK_BYTES.store(0, Ordering::Relaxed);
    }

    #[test]
    fn peak_starts_at_zero() {
        let _g = lock();
        reset_all();
        assert_eq!(peak_bytes(), 0);
        assert_eq!(current_bytes(), 0);
    }

    #[test]
    fn alloc_bumps_peak_at_least_by_size() {
        let _g = lock();
        reset_all();
        let alloc = TrackingHeap::new(SystemAlloc);
        let layout = Layout::from_size_align(128, 8).unwrap();
        // SAFETY: layout is non-zero and well-formed.
        let p = unsafe { alloc.alloc(layout) };
        assert!(!p.is_null());
        assert!(peak_bytes() >= 128);
        assert!(current_bytes() >= 128);
        // SAFETY: pointer was returned from this same allocator
        // with this same layout, has not been freed.
        unsafe { alloc.dealloc(p, layout) };
        assert_eq!(current_bytes(), 0);
        // Peak does NOT decrease on dealloc — it is a high-water
        // mark.
        assert!(peak_bytes() >= 128);
    }

    #[test]
    fn reset_peak_drops_high_water_to_current() {
        let _g = lock();
        reset_all();
        let alloc = TrackingHeap::new(SystemAlloc);
        let layout = Layout::from_size_align(64, 8).unwrap();
        // SAFETY: see above.
        let p = unsafe { alloc.alloc(layout) };
        assert!(peak_bytes() >= 64);
        // SAFETY: see above.
        unsafe { alloc.dealloc(p, layout) };
        assert_eq!(current_bytes(), 0);
        assert!(peak_bytes() >= 64);
        reset_peak();
        // After reset, peak == current (== 0 here).
        assert_eq!(peak_bytes(), 0);
    }

    #[test]
    fn peak_tracks_max_across_allocations() {
        let _g = lock();
        reset_all();
        let alloc = TrackingHeap::new(SystemAlloc);
        let l = Layout::from_size_align(256, 8).unwrap();
        // SAFETY: see above for all alloc/dealloc calls in this test.
        let p1 = unsafe { alloc.alloc(l) };
        let p2 = unsafe { alloc.alloc(l) };
        // Two live allocations: at least 512 bytes.
        assert!(peak_bytes() >= 512);
        unsafe { alloc.dealloc(p1, l) };
        // Peak remembers the high water mark.
        assert!(peak_bytes() >= 512);
        // Current dropped by 256.
        assert!(current_bytes() < 512);
        unsafe { alloc.dealloc(p2, l) };
    }

    #[test]
    fn known_size_struct_allocation_lifts_peak() {
        // Acceptance criterion: "allocate a known structure;
        // assert peak_bytes() >= size_of::<Structure>()".
        let _g = lock();
        reset_all();
        struct Sample {
            _payload: [u64; 16], // 128 bytes
        }
        let _v: alloc_crate::boxed::Box<Sample> =
            alloc_crate::boxed::Box::new(Sample { _payload: [0; 16] });
        // We can't guarantee the BOXED allocation went through OUR
        // TrackingHeap (the test harness uses the host allocator),
        // so this assertion targets the documented contract: a
        // live allocation through the wrapper bumps the peak.
        let alloc = TrackingHeap::new(SystemAlloc);
        let layout = Layout::new::<Sample>();
        // SAFETY: well-formed layout.
        let p = unsafe { alloc.alloc(layout) };
        assert!(!p.is_null());
        assert!(peak_bytes() >= core::mem::size_of::<Sample>());
        // SAFETY: see above.
        unsafe { alloc.dealloc(p, layout) };
    }

    #[test]
    fn realloc_grow_increases_peak() {
        let _g = lock();
        reset_all();
        let alloc = TrackingHeap::new(SystemAlloc);
        let l1 = Layout::from_size_align(64, 8).unwrap();
        // SAFETY: see above.
        let p = unsafe { alloc.alloc(l1) };
        assert!(peak_bytes() >= 64);
        // SAFETY: pointer + layout came from this allocator; new
        // size 256 > 0.
        let p2 = unsafe { alloc.realloc(p, l1, 256) };
        assert!(!p2.is_null());
        assert!(peak_bytes() >= 256);
        let l2 = Layout::from_size_align(256, 8).unwrap();
        // SAFETY: layout describes the current allocation size.
        unsafe { alloc.dealloc(p2, l2) };
    }
}

/// Without the `alloc` feature, `peak_bytes` and `reset_peak` are
/// still available but inert — callers can write portable code
/// that compiles in both postures. Verify the inert behaviour.
#[cfg(all(test, not(feature = "alloc")))]
mod inert_tests {
    use super::*;

    #[test]
    fn peak_is_zero_without_alloc_feature() {
        // Counters are still defined — they're just never bumped.
        assert_eq!(peak_bytes(), 0);
        reset_peak();
        assert_eq!(peak_bytes(), 0);
    }
}
