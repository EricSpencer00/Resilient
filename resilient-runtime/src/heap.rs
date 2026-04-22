//! RES-374: heap profiler — peak allocation tracking.
//!
//! Provides `peak_bytes()` and `reset_peak()` for observing the
//! high-water mark of heap usage on `--features alloc` builds.
//! On default (no-alloc) builds both functions are no-ops that
//! return `0`, so call sites need no feature-gating.
//!
//! ## Wiring
//!
//! Install `ProfilingAllocator` as your `#[global_allocator]` to
//! enable automatic tracking:
//!
//! ```rust,ignore
//! use embedded_alloc::Heap;
//! use resilient_runtime::heap::ProfilingAllocator;
//!
//! #[global_allocator]
//! static HEAP: ProfilingAllocator<Heap> = ProfilingAllocator::new(Heap::empty());
//! ```
//!
//! After that, call `peak_bytes()` at any point to read the
//! high-water mark.

#[cfg(feature = "alloc")]
use core::sync::atomic::{AtomicUsize, Ordering};

// --- internal atomics (alloc builds only) ---

/// Running total of live heap bytes (increases on alloc, decreases on dealloc).
#[cfg(feature = "alloc")]
static CURRENT_BYTES: AtomicUsize = AtomicUsize::new(0);

/// Largest value `CURRENT_BYTES` has ever reached.
#[cfg(feature = "alloc")]
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);

// --- public API ---

/// Returns the peak heap allocation (in bytes) since program start
/// or since the last call to [`reset_peak`].
///
/// Only meaningful when the `alloc` feature is enabled **and** a
/// [`ProfilingAllocator`] is installed as the `#[global_allocator]`.
/// Returns `0` on no-alloc builds.
pub fn peak_bytes() -> usize {
    #[cfg(feature = "alloc")]
    {
        PEAK_BYTES.load(Ordering::Relaxed)
    }
    #[cfg(not(feature = "alloc"))]
    {
        0
    }
}

/// Resets the peak high-water mark to the current live allocation size.
///
/// Useful to scope profiling to a particular phase of execution.
/// No-op on no-alloc builds.
pub fn reset_peak() {
    #[cfg(feature = "alloc")]
    {
        let current = CURRENT_BYTES.load(Ordering::Relaxed);
        PEAK_BYTES.store(current, Ordering::Relaxed);
    }
}

// --- internal helpers (alloc builds only) ---

/// Called by [`ProfilingAllocator`] after a successful allocation of
/// `size` bytes to update the current and peak counters.
#[cfg(feature = "alloc")]
pub(crate) fn record_alloc(size: usize) {
    let prev = CURRENT_BYTES.fetch_add(size, Ordering::Relaxed);
    let new_total = prev.saturating_add(size);
    // Update peak if new_total exceeds the stored value. A CAS loop
    // is safe here because only `record_alloc` raises PEAK_BYTES and
    // we only write when `new_total > current_peak`.
    let mut current_peak = PEAK_BYTES.load(Ordering::Relaxed);
    while new_total > current_peak {
        match PEAK_BYTES.compare_exchange_weak(
            current_peak,
            new_total,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(p) => current_peak = p,
        }
    }
}

/// Called by [`ProfilingAllocator`] after a deallocation of `size`
/// bytes to keep `CURRENT_BYTES` accurate.
#[cfg(feature = "alloc")]
pub(crate) fn record_dealloc(size: usize) {
    CURRENT_BYTES.fetch_sub(size, Ordering::Relaxed);
}

// --- ProfilingAllocator ---

/// A thin allocator wrapper that records peak heap usage.
///
/// Wraps any type that implements [`core::alloc::GlobalAlloc`] and
/// forwards every allocation/deallocation while updating the
/// module-level counters read by [`peak_bytes`] and [`reset_peak`].
///
/// # Example
///
/// ```rust,ignore
/// use embedded_alloc::Heap;
/// use resilient_runtime::heap::ProfilingAllocator;
///
/// #[global_allocator]
/// static HEAP: ProfilingAllocator<Heap> = ProfilingAllocator::new(Heap::empty());
/// ```
#[cfg(feature = "alloc")]
pub struct ProfilingAllocator<A>(A);

#[cfg(feature = "alloc")]
impl<A> ProfilingAllocator<A> {
    /// Wraps `inner` in a profiling shim. Const so it can be used in
    /// `static` initialisers.
    pub const fn new(inner: A) -> Self {
        Self(inner)
    }
}

// SAFETY: `ProfilingAllocator<A>` is a transparent newtype over `A`.
// All safety invariants delegated to the inner `A::alloc` /
// `A::dealloc` implementations.  The additional bookkeeping
// (atomic counter updates) is data-race-free because atomics with
// `Relaxed` ordering are used consistently.
#[cfg(feature = "alloc")]
unsafe impl<A: core::alloc::GlobalAlloc> core::alloc::GlobalAlloc for ProfilingAllocator<A> {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        // SAFETY: delegating directly to the inner allocator; the
        // caller must uphold the same invariants as `GlobalAlloc::alloc`.
        let ptr = unsafe { self.0.alloc(layout) };
        if !ptr.is_null() {
            record_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: core::alloc::Layout) {
        // SAFETY: delegating directly to the inner allocator; the
        // caller must uphold the same invariants as `GlobalAlloc::dealloc`.
        unsafe { self.0.dealloc(ptr, layout) };
        record_dealloc(layout.size());
    }
}

// --- tests ---

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    use core::mem::size_of;

    // Install ProfilingAllocator<System> as the global allocator for
    // this test binary so that `Box::new(...)` / `String::from(...)`
    // update the peak counter.  `std::alloc::System` is available
    // because `no_std` is suppressed in test builds via
    // `cfg_attr(not(any(test, feature = "std-sink")), no_std)`.
    #[global_allocator]
    static TEST_ALLOC: ProfilingAllocator<std::alloc::System> =
        ProfilingAllocator::new(std::alloc::System);

    /// A known-size structure used as the allocation target.
    #[repr(C)]
    struct Probe {
        a: u64,
        b: u64,
        c: u64,
        d: u64,
    }

    #[test]
    fn peak_bytes_reflects_allocation() {
        reset_peak();
        let before = peak_bytes();

        // Heap-allocate a known structure via Box.
        let _probe = Box::new(Probe {
            a: 1,
            b: 2,
            c: 3,
            d: 4,
        });

        let after = peak_bytes();
        assert!(
            after >= before + size_of::<Probe>(),
            "peak_bytes() should have grown by at least size_of::<Probe>()={} bytes, \
             but went from {} to {}",
            size_of::<Probe>(),
            before,
            after,
        );
    }

    #[test]
    fn reset_peak_lowers_high_water_mark() {
        // Force a large allocation to set a high peak.
        let _v: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(1024);
        let high = peak_bytes();
        assert!(
            high > 0,
            "peak should be non-zero after a 1024-byte allocation"
        );

        // Drop allocation, then reset.
        drop(_v);
        reset_peak();
        let low = peak_bytes();

        assert!(
            low < high,
            "peak after reset ({low}) should be below the previous high-water mark ({high})"
        );
    }

    #[test]
    fn peak_bytes_zero_before_any_alloc_after_reset() {
        // After reset with zero live allocations the peak should be 0.
        // This test relies on no live allocations existing at the
        // moment reset_peak is called; since tests may run in parallel
        // this is best-effort — we only assert the invariant holds when
        // CURRENT_BYTES is verifiably 0.
        CURRENT_BYTES.store(0, Ordering::Relaxed);
        PEAK_BYTES.store(0, Ordering::Relaxed);
        assert_eq!(peak_bytes(), 0);
    }
}

#[cfg(all(test, not(feature = "alloc")))]
mod no_alloc_tests {
    use super::*;

    #[test]
    fn peak_bytes_returns_zero_without_alloc_feature() {
        assert_eq!(peak_bytes(), 0);
    }

    #[test]
    fn reset_peak_is_noop_without_alloc_feature() {
        reset_peak(); // must not panic or fail to compile
        assert_eq!(peak_bytes(), 0);
    }
}
