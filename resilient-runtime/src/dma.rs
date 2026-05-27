//! RES-2594: DMA descriptor chains for zero-copy embedded transfers.
//!
//! Direct Memory Access (DMA) lets a peripheral move bytes between
//! memory regions without spending CPU cycles per byte. The hardware
//! consumes a *descriptor* — a struct that says "copy `length` bytes
//! from `source` to `dest`, then follow `next` to find the next job"
//! — and walks the linked list until `next` is null. ADC sampling
//! into a circular buffer, SPI flash bulk reads, UART back-to-back
//! frame TX all use this shape.
//!
//! # What this module provides
//!
//! - [`DmaDescriptor`]: the linked-list node the hardware sees. Four
//!   fields, packed C-layout, every byte deterministic so a real DMA
//!   engine could load it (host-side tests don't exercise hardware —
//!   the layout discipline is for the embedded targets to plug in).
//! - [`DmaChain`]: an owning, fixed-capacity arena that hands out
//!   descriptor IDs and stitches `next` pointers together. Lets
//!   firmware build a chain without `alloc`, and the type system
//!   keeps every borrow consistent.
//! - [`DmaTransfer`]: the consumed handle you hand to
//!   [`dma_start_transfer`]. The chain moves into the handle, so
//!   no one can mutate descriptors mid-flight.
//!
//! # Why no_std?
//!
//! Same posture as the rest of `resilient-runtime`: the default
//! feature set is `#![no_std]` with zero heap allocation. The arena
//! is a stack/static `[DmaDescriptor; N]` — capacity picked at
//! type-construction time. There is no allocator pressure, no
//! recursive type, and the only runtime checks are alignment and
//! length-range — both of which a typechecker can lift into
//! compile-time errors when the operand is a literal.
//!
//! # Linear types
//!
//! `resilient/src/linear.rs` carries a `linear` annotation through
//! the Resilient surface language. A `linear` DMA buffer is consumed
//! at most once — exactly what you want for a transfer whose
//! hardware semantics overlap reads and writes. The API mirror on
//! this side is movement, not borrowing: [`DmaChain::append`] takes
//! `&mut self` (only one chain builder at a time) and
//! [`DmaChain::start`] takes `self` by value, returning a
//! [`DmaTransfer`] that owns the chain for the duration of the
//! transfer. Once you call `start`, the chain is unreachable; the
//! compiler enforces no further mutation. That's the runtime half
//! of the linearity story — the language half is in `linear.rs`.
//!
//! # Compile-time validation
//!
//! Two kinds of validity, both checked at construction:
//!
//! 1. **Alignment.** Source and destination addresses must be aligned
//!    to the DMA word width (1, 2, or 4 bytes — picked when you build
//!    the descriptor). Misaligned access fires a `BusFault` on
//!    Cortex-M; on RISC-V it traps. We reject at the API boundary so
//!    a misconfigured chain can never reach the hardware.
//! 2. **Length bounds.** The hardware can't transfer 0 bytes (no-op
//!    descriptors waste a slot and silently confuse loop counters)
//!    nor more than `u16::MAX` bytes in a single descriptor — the
//!    DMA1 controller on STM32F4 uses a 16-bit NDTR register. Both
//!    bounds are checked.
//!
//! Both checks live in [`DmaDescriptor::new`] and return a typed
//! [`DmaError`] enum so callers can distinguish (and the typechecker
//! can lift them to compile-time errors for constant inputs).

use core::ptr;

/// Capacity ceiling we enforce on every [`DmaChain`]. Picked to fit
/// a realistic ADC-double-buffer / SPI-burst workload without
/// blowing the stack on Cortex-M0+. Bumping this is free, but every
/// stack frame holding a `DmaChain<N>` grows by
/// `N * size_of::<DmaDescriptor>()`, so callers should pick the
/// smallest N that fits their workload.
pub const DMA_CHAIN_MAX_CAPACITY: usize = 256;

/// DMA word width. Determines alignment requirements on `source` and
/// `dest`, and the size in bytes of each transfer beat.
///
/// Most peripherals only support a subset (UART = byte, SPI = byte or
/// halfword, ADC = halfword or word). Build the chain with the
/// width the *peripheral* dictates; the engine will read/write that
/// many bytes per beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaWidth {
    /// 8-bit beats. Alignment requirement: 1 (any byte).
    Byte,
    /// 16-bit beats. Alignment requirement: 2 (halfword-aligned).
    HalfWord,
    /// 32-bit beats. Alignment requirement: 4 (word-aligned).
    Word,
}

impl DmaWidth {
    /// Required alignment in bytes for `source` and `dest` addresses.
    #[inline]
    pub const fn alignment(self) -> usize {
        match self {
            DmaWidth::Byte => 1,
            DmaWidth::HalfWord => 2,
            DmaWidth::Word => 4,
        }
    }

    /// Bytes per transfer beat. Same value as [`Self::alignment`]
    /// today, but exposed as a separate accessor so future widths
    /// (BurstWord = 16 with 4-byte alignment) can diverge cleanly.
    #[inline]
    pub const fn bytes_per_beat(self) -> usize {
        self.alignment()
    }
}

/// Errors produced when constructing or extending a DMA chain. Every
/// error carries enough context for a diagnostic; none of them are
/// recoverable at runtime — a misconfigured descriptor is a programmer
/// error and the right response is to fail the build (the typechecker
/// can lift these to compile-time on literal inputs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaError {
    /// `source` address is not aligned for the selected width.
    /// Carries the bad address and the expected alignment.
    SourceMisaligned { addr: usize, required: usize },
    /// `dest` address is not aligned for the selected width.
    DestMisaligned { addr: usize, required: usize },
    /// `length` is zero. Zero-byte descriptors are a no-op that wastes
    /// a chain slot — we reject them outright.
    ZeroLength,
    /// `length` exceeds the 16-bit NDTR register width. STM32F4 et al.
    /// can only count down a 16-bit counter, so a single descriptor
    /// caps at 65 535 bytes. Split into multiple descriptors.
    LengthTooLarge { length: usize, max: usize },
    /// The chain is already at capacity. Pick a larger `N` when
    /// constructing the [`DmaChain`].
    ChainFull { capacity: usize },
}

/// Maximum bytes a single descriptor can transfer. Matches the
/// STM32F4 DMA1 NDTR register width (16 bits). RISC-V DMA-CH0 has the
/// same limit. Larger transfers split across multiple descriptors.
pub const DMA_MAX_LENGTH: usize = u16::MAX as usize;

/// A single DMA descriptor — the linked-list node the hardware reads.
///
/// Layout is `#[repr(C)]` so the offsets are deterministic and a
/// real DMA engine could be pointed at the struct. Field order
/// (`source, dest, length, next`) matches the most common DMA
/// controller layouts (STM32 DMA, NXP eDMA — both put the address
/// pair first, then the transfer counter, then the next-link
/// pointer).
///
/// `next` is a raw pointer rather than `Option<&DmaDescriptor>` for
/// two reasons:
///
/// 1. The hardware reads a fixed memory layout. A reference would
///    impose Rust's niche-optimisation rules and we'd lose the
///    "null = end of chain" sentinel.
/// 2. The chain is a linked list whose nodes live inside a
///    [`DmaChain`] arena. The arena keeps them alive; a borrow
///    would conflict with the `&mut self` of [`DmaChain::append`].
///
/// We never deref `next` from Rust code (the hardware does that).
/// Host-side tests walk the chain via the arena's index API instead.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DmaDescriptor {
    /// Source address — first byte the engine reads.
    pub source: usize,
    /// Destination address — first byte the engine writes.
    pub dest: usize,
    /// Number of bytes to transfer. 1..=[`DMA_MAX_LENGTH`].
    pub length: u32,
    /// Pointer to the next descriptor, or null to terminate the chain.
    /// `*const` (not `*mut`) — the engine reads but never writes the
    /// `next` field of an in-flight descriptor.
    pub next: *const DmaDescriptor,
}

// SAFETY: `DmaDescriptor` is plain old data (four usize/u32 fields).
// `next` is a raw pointer, which doesn't implement Send/Sync by
// default. But the descriptor is only "live" while the owning
// `DmaTransfer` exists, and that handle isn't shareable across
// threads either. We don't expose interior mutability and never
// follow the pointer from Rust code, so the Send/Sync bounds are
// safe to assert — they let callers stash a built chain in a
// `static` for repeated reuse, which is the embedded use case.
unsafe impl Send for DmaDescriptor {}
unsafe impl Sync for DmaDescriptor {}

impl DmaDescriptor {
    /// Construct a single descriptor, checking alignment and length
    /// bounds. The resulting descriptor has `next = null`, marking
    /// it as the chain tail. Use [`DmaChain::append`] to thread
    /// descriptors together.
    ///
    /// # Errors
    ///
    /// - [`DmaError::SourceMisaligned`] / [`DmaError::DestMisaligned`]
    ///   if either address isn't aligned to `width.alignment()`.
    /// - [`DmaError::ZeroLength`] if `length == 0`.
    /// - [`DmaError::LengthTooLarge`] if `length > DMA_MAX_LENGTH`.
    pub fn new(
        source: usize,
        dest: usize,
        length: usize,
        width: DmaWidth,
    ) -> Result<Self, DmaError> {
        let align = width.alignment();
        if !source.is_multiple_of(align) {
            return Err(DmaError::SourceMisaligned {
                addr: source,
                required: align,
            });
        }
        if !dest.is_multiple_of(align) {
            return Err(DmaError::DestMisaligned {
                addr: dest,
                required: align,
            });
        }
        if length == 0 {
            return Err(DmaError::ZeroLength);
        }
        if length > DMA_MAX_LENGTH {
            return Err(DmaError::LengthTooLarge {
                length,
                max: DMA_MAX_LENGTH,
            });
        }
        Ok(Self {
            source,
            dest,
            length: length as u32,
            next: ptr::null(),
        })
    }
}

/// Builder-only constructor matching the Resilient-surface name.
/// Mirrors the `dma_descriptor_new` builtin the language exposes.
/// Thin wrapper around [`DmaDescriptor::new`] — kept as a free
/// function so the FFI dispatcher can hand it a stable symbol name.
#[inline]
pub fn dma_descriptor_new(
    source: usize,
    dest: usize,
    length: usize,
    width: DmaWidth,
) -> Result<DmaDescriptor, DmaError> {
    DmaDescriptor::new(source, dest, length, width)
}

/// Fixed-capacity arena of descriptors stitched into a linked list.
///
/// The arena owns its storage as `[DmaDescriptor; N]`, so there is
/// no allocator dependency. `N` is the *capacity* — the maximum
/// number of descriptors a single chain can hold. `len()` tracks
/// how many are currently in use. Building a chain is push-only;
/// once a descriptor is linked, its `next` field is immutable.
///
/// # Why fixed N
///
/// On embedded targets you size your DMA workload at firmware-build
/// time. A growing `Vec` would need an allocator and introduces
/// realloc-induced pointer invalidation that would silently corrupt
/// `next` links mid-transfer. Pinning the storage to the type makes
/// the layout deterministic.
pub struct DmaChain<const N: usize> {
    /// Descriptor storage. The arena writes through `MaybeUninit` —
    /// uninitialised entries are never observed.
    descriptors: [core::mem::MaybeUninit<DmaDescriptor>; N],
    /// Number of initialised descriptors at the head of `descriptors`.
    len: usize,
}

impl<const N: usize> Default for DmaChain<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> DmaChain<N> {
    /// Construct an empty chain.
    ///
    /// `N` must be in `1..=DMA_CHAIN_MAX_CAPACITY` for the resulting
    /// chain to be usable — see [`Self::is_valid_capacity`]. The
    /// constructor itself is infallible (so it can be `const`), but
    /// [`Self::append`] returns [`DmaError::ChainFull`] when an
    /// invalid capacity is paired with the first append.
    pub const fn new() -> Self {
        Self {
            descriptors: [const { core::mem::MaybeUninit::uninit() }; N],
            len: 0,
        }
    }

    /// True iff `N` is a usable chain capacity. Const so the
    /// typechecker can fold it on literal inputs and reject
    /// `DmaChain<0>` at compile time when the chain is actually used.
    #[inline]
    pub const fn is_valid_capacity() -> bool {
        N >= 1 && N <= DMA_CHAIN_MAX_CAPACITY
    }

    /// Number of descriptors currently in the chain.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True iff the chain holds zero descriptors.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Maximum descriptors this chain can hold (= `N`).
    #[inline]
    pub const fn capacity(&self) -> usize {
        N
    }

    /// Append a descriptor to the chain, stitching the previous
    /// tail's `next` pointer to the new entry.
    ///
    /// # Errors
    ///
    /// - [`DmaError::ChainFull`] if the chain already holds `N`
    ///   descriptors, or `N` is outside the valid capacity range.
    pub fn append(&mut self, desc: DmaDescriptor) -> Result<(), DmaError> {
        if !Self::is_valid_capacity() {
            return Err(DmaError::ChainFull { capacity: N });
        }
        if self.len >= N {
            return Err(DmaError::ChainFull { capacity: N });
        }
        // Reset incoming descriptor's `next` to null — we own the
        // linking decision, not the caller. Otherwise a caller
        // could smuggle a pointer into someone else's chain.
        let mut desc = desc;
        desc.next = ptr::null();
        self.descriptors[self.len].write(desc);
        let new_index = self.len;
        self.len += 1;
        // Patch the previous tail to point at the new entry. The
        // pointer is valid as long as the chain isn't moved, which
        // is the invariant `DmaTransfer` enforces by taking the
        // chain by value (or `Pin` for callers who put the chain
        // in a static).
        if new_index > 0 {
            // SAFETY: index 0..new_index-1 is initialised (the loop
            // invariant of `len`) and the slot at new_index is the
            // one we just wrote. Read+write through MaybeUninit is
            // legal for initialised entries.
            unsafe {
                let new_ptr = self.descriptors[new_index].assume_init_ref() as *const DmaDescriptor;
                let prev = self.descriptors[new_index - 1].assume_init_mut();
                prev.next = new_ptr;
            }
        }
        Ok(())
    }

    /// Borrow the descriptor at `index`. Returns `None` if the
    /// index is past the chain's `len()`. Used by host-side tests
    /// to walk the chain without dereferencing raw pointers.
    pub fn get(&self, index: usize) -> Option<&DmaDescriptor> {
        if index >= self.len {
            return None;
        }
        // SAFETY: index < self.len, and the invariant on `len` is
        // that the first `len` slots are initialised.
        Some(unsafe { self.descriptors[index].assume_init_ref() })
    }

    /// Head pointer — what you'd hand a DMA controller's `PADR`
    /// (peripheral address-of-descriptor) register. Null if the
    /// chain is empty.
    ///
    /// Returned as `*const DmaDescriptor` (the engine reads it, never
    /// writes it). The pointer is valid for as long as `self` is
    /// alive and unmoved.
    pub fn head_ptr(&self) -> *const DmaDescriptor {
        if self.len == 0 {
            ptr::null()
        } else {
            // SAFETY: len >= 1, so index 0 is initialised.
            unsafe { self.descriptors[0].assume_init_ref() as *const DmaDescriptor }
        }
    }

    /// Consume the chain and produce a [`DmaTransfer`] handle ready
    /// to hand to the hardware. After this call the chain is
    /// unreachable — exactly the linearity guarantee a `linear`
    /// DMA buffer needs.
    pub fn start(self) -> DmaTransfer<N> {
        DmaTransfer { chain: self }
    }
}

/// Free-function builder mirroring the Resilient-surface name.
/// Thin wrapper around [`DmaChain::append`] kept so the FFI
/// dispatcher has a stable symbol.
#[inline]
pub fn dma_chain_append<const N: usize>(
    chain: &mut DmaChain<N>,
    desc: DmaDescriptor,
) -> Result<(), DmaError> {
    chain.append(desc)
}

/// A consumed chain ready for DMA execution.
///
/// Holds the chain by value so no one can mutate descriptors while
/// the engine reads them. Drop this when the transfer is complete
/// (the storage is freed when the handle goes out of scope).
///
/// We deliberately do NOT implement `Clone`/`Copy` on this type —
/// duplicating a transfer would alias the underlying descriptor
/// storage, defeating the linearity guarantee.
pub struct DmaTransfer<const N: usize> {
    chain: DmaChain<N>,
}

impl<const N: usize> DmaTransfer<N> {
    /// Number of descriptors in the underlying chain.
    #[inline]
    pub const fn descriptor_count(&self) -> usize {
        self.chain.len()
    }

    /// Head pointer — hand this to the DMA controller's address
    /// register. Stays valid for the lifetime of `self`.
    pub fn head_ptr(&self) -> *const DmaDescriptor {
        self.chain.head_ptr()
    }

    /// Total bytes the entire chain will transfer. Useful for
    /// progress reporting and for the test harness to assert the
    /// chain matches the workload.
    pub fn total_bytes(&self) -> u64 {
        let mut total: u64 = 0;
        for i in 0..self.chain.len() {
            if let Some(d) = self.chain.get(i) {
                total = total.saturating_add(d.length as u64);
            }
        }
        total
    }

    /// Borrow a descriptor by index. Host-side test affordance —
    /// the embedded path uses `head_ptr` and lets the hardware walk
    /// the chain.
    pub fn descriptor(&self, index: usize) -> Option<&DmaDescriptor> {
        self.chain.get(index)
    }
}

/// Free-function builder mirroring the Resilient-surface name. The
/// real embedded implementation would also poke the controller's
/// enable register; the host-side version just consumes the chain
/// and returns the transfer handle so unit tests can inspect the
/// resulting linked list.
#[inline]
pub fn dma_start_transfer<const N: usize>(chain: DmaChain<N>) -> DmaTransfer<N> {
    chain.start()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Static buffers used as DMA source/dest in tests. `repr(align)`
    // gives us a known-aligned address so the alignment checks in
    // `DmaDescriptor::new` are exercising real values.
    #[repr(align(4))]
    struct AlignedBuf([u8; 64]);

    static SRC: AlignedBuf = AlignedBuf([0u8; 64]);
    static DST: AlignedBuf = AlignedBuf([0u8; 64]);

    fn src_addr() -> usize {
        SRC.0.as_ptr() as usize
    }
    fn dst_addr() -> usize {
        DST.0.as_ptr() as usize
    }

    // ---------- DmaDescriptor::new — alignment / length checks ----------

    #[test]
    fn descriptor_new_byte_width_accepts_any_alignment() {
        let d = DmaDescriptor::new(src_addr() + 1, dst_addr() + 3, 17, DmaWidth::Byte).unwrap();
        assert_eq!(d.length, 17);
        assert!(d.next.is_null());
    }

    #[test]
    fn descriptor_new_word_width_rejects_unaligned_source() {
        let err = DmaDescriptor::new(src_addr() + 1, dst_addr(), 16, DmaWidth::Word).unwrap_err();
        match err {
            DmaError::SourceMisaligned { required, .. } => assert_eq!(required, 4),
            other => panic!("expected SourceMisaligned, got {other:?}"),
        }
    }

    #[test]
    fn descriptor_new_word_width_rejects_unaligned_dest() {
        let err = DmaDescriptor::new(src_addr(), dst_addr() + 2, 16, DmaWidth::Word).unwrap_err();
        match err {
            DmaError::DestMisaligned { required, .. } => assert_eq!(required, 4),
            other => panic!("expected DestMisaligned, got {other:?}"),
        }
    }

    #[test]
    fn descriptor_new_halfword_width_requires_2_byte_alignment() {
        // Odd source — should fail.
        let err =
            DmaDescriptor::new(src_addr() + 1, dst_addr(), 8, DmaWidth::HalfWord).unwrap_err();
        assert!(matches!(
            err,
            DmaError::SourceMisaligned { required: 2, .. }
        ));
        // Even source — should succeed.
        let d = DmaDescriptor::new(src_addr() + 2, dst_addr() + 4, 8, DmaWidth::HalfWord).unwrap();
        assert_eq!(d.length, 8);
    }

    #[test]
    fn descriptor_new_rejects_zero_length() {
        let err = DmaDescriptor::new(src_addr(), dst_addr(), 0, DmaWidth::Byte).unwrap_err();
        assert_eq!(err, DmaError::ZeroLength);
    }

    #[test]
    fn descriptor_new_rejects_oversized_length() {
        let err = DmaDescriptor::new(src_addr(), dst_addr(), DMA_MAX_LENGTH + 1, DmaWidth::Byte)
            .unwrap_err();
        match err {
            DmaError::LengthTooLarge { max, .. } => assert_eq!(max, DMA_MAX_LENGTH),
            other => panic!("expected LengthTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn descriptor_new_accepts_max_length() {
        let d = DmaDescriptor::new(src_addr(), dst_addr(), DMA_MAX_LENGTH, DmaWidth::Byte).unwrap();
        assert_eq!(d.length as usize, DMA_MAX_LENGTH);
    }

    // ---------- DmaWidth helpers ----------

    #[test]
    fn dma_width_alignment_matches_bytes_per_beat() {
        assert_eq!(DmaWidth::Byte.alignment(), 1);
        assert_eq!(DmaWidth::Byte.bytes_per_beat(), 1);
        assert_eq!(DmaWidth::HalfWord.alignment(), 2);
        assert_eq!(DmaWidth::HalfWord.bytes_per_beat(), 2);
        assert_eq!(DmaWidth::Word.alignment(), 4);
        assert_eq!(DmaWidth::Word.bytes_per_beat(), 4);
    }

    // ---------- DmaChain — capacity, append, linking ----------

    #[test]
    fn chain_starts_empty() {
        let chain: DmaChain<4> = DmaChain::new();
        assert_eq!(chain.len(), 0);
        assert!(chain.is_empty());
        assert_eq!(chain.capacity(), 4);
        assert!(chain.head_ptr().is_null());
    }

    #[test]
    fn chain_append_increments_len() {
        let mut chain: DmaChain<4> = DmaChain::new();
        let d = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        chain.append(d).unwrap();
        assert_eq!(chain.len(), 1);
        assert!(!chain.is_empty());
    }

    #[test]
    fn chain_links_descriptors_through_next_pointer() {
        let mut chain: DmaChain<4> = DmaChain::new();
        let d1 = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        let d2 = DmaDescriptor::new(src_addr() + 8, dst_addr() + 8, 16, DmaWidth::Word).unwrap();
        let d3 = DmaDescriptor::new(src_addr() + 24, dst_addr() + 24, 4, DmaWidth::Word).unwrap();
        chain.append(d1).unwrap();
        chain.append(d2).unwrap();
        chain.append(d3).unwrap();
        assert_eq!(chain.len(), 3);
        // First two descriptors should point at their successor;
        // the tail's `next` is null.
        let n0 = chain.get(0).unwrap();
        let n1 = chain.get(1).unwrap();
        let n2 = chain.get(2).unwrap();
        assert_eq!(n0.next, n1 as *const _);
        assert_eq!(n1.next, n2 as *const _);
        assert!(n2.next.is_null());
    }

    #[test]
    fn chain_append_resets_external_next_pointer() {
        // A caller can't smuggle in a `next` pointer — the chain
        // owns linking. Build a descriptor that ALREADY has a
        // (fake) next pointer and append; the chain should null
        // it before stitching.
        let mut chain: DmaChain<2> = DmaChain::new();
        let mut d1 = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        // Set an arbitrary non-null pointer; the chain must clear it.
        d1.next = src_addr() as *const DmaDescriptor;
        chain.append(d1).unwrap();
        // Single-element chain: head's next must be null.
        assert!(chain.get(0).unwrap().next.is_null());
    }

    #[test]
    fn chain_append_fills_to_capacity() {
        let mut chain: DmaChain<2> = DmaChain::new();
        for offset in [0usize, 8] {
            let d = DmaDescriptor::new(src_addr() + offset, dst_addr() + offset, 4, DmaWidth::Word)
                .unwrap();
            chain.append(d).unwrap();
        }
        assert_eq!(chain.len(), 2);
        // Third append must fail.
        let d3 = DmaDescriptor::new(src_addr(), dst_addr(), 4, DmaWidth::Word).unwrap();
        let err = chain.append(d3).unwrap_err();
        assert_eq!(err, DmaError::ChainFull { capacity: 2 });
    }

    #[test]
    fn chain_head_ptr_points_at_first_descriptor() {
        let mut chain: DmaChain<2> = DmaChain::new();
        let d = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        chain.append(d).unwrap();
        let head = chain.head_ptr();
        let first = chain.get(0).unwrap();
        assert_eq!(head, first as *const _);
    }

    #[test]
    fn chain_get_out_of_bounds_returns_none() {
        let mut chain: DmaChain<4> = DmaChain::new();
        chain
            .append(DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap())
            .unwrap();
        assert!(chain.get(1).is_none());
        assert!(chain.get(100).is_none());
    }

    // ---------- Capacity validity ----------

    #[test]
    fn chain_zero_capacity_is_invalid() {
        // A zero-cap chain is constructible (constness lets us
        // skip the check) but unusable.
        let mut chain: DmaChain<0> = DmaChain::new();
        assert!(!DmaChain::<0>::is_valid_capacity());
        let d = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        assert_eq!(
            chain.append(d).unwrap_err(),
            DmaError::ChainFull { capacity: 0 }
        );
    }

    #[test]
    fn chain_oversized_capacity_is_invalid() {
        // 257 > DMA_CHAIN_MAX_CAPACITY = 256.
        assert!(!DmaChain::<257>::is_valid_capacity());
        assert!(DmaChain::<256>::is_valid_capacity());
        assert!(DmaChain::<1>::is_valid_capacity());
    }

    // ---------- DmaTransfer — linearity / handoff ----------

    #[test]
    fn start_transfer_preserves_descriptor_count() {
        let mut chain: DmaChain<3> = DmaChain::new();
        for offset in [0usize, 8, 16] {
            chain
                .append(
                    DmaDescriptor::new(src_addr() + offset, dst_addr() + offset, 4, DmaWidth::Word)
                        .unwrap(),
                )
                .unwrap();
        }
        let transfer = chain.start();
        assert_eq!(transfer.descriptor_count(), 3);
        assert_eq!(transfer.total_bytes(), 12);
    }

    #[test]
    fn start_transfer_keeps_head_pointer_valid() {
        let mut chain: DmaChain<2> = DmaChain::new();
        chain
            .append(DmaDescriptor::new(src_addr(), dst_addr(), 16, DmaWidth::Word).unwrap())
            .unwrap();
        let transfer = chain.start();
        let head = transfer.head_ptr();
        assert!(!head.is_null());
        // SAFETY: the chain (now inside `transfer`) is alive, so head
        // points at a live descriptor. The test never aliases it
        // mutably.
        let head_desc = unsafe { &*head };
        assert_eq!(head_desc.length, 16);
    }

    #[test]
    fn transfer_descriptor_walks_chain() {
        let mut chain: DmaChain<3> = DmaChain::new();
        for offset in [0usize, 8, 16] {
            chain
                .append(
                    DmaDescriptor::new(src_addr() + offset, dst_addr() + offset, 4, DmaWidth::Word)
                        .unwrap(),
                )
                .unwrap();
        }
        let transfer = chain.start();
        for i in 0..3 {
            let d = transfer.descriptor(i).unwrap();
            assert_eq!(d.length, 4);
        }
        assert!(transfer.descriptor(3).is_none());
    }

    // ---------- Free-function builders (FFI stable names) ----------

    #[test]
    fn builder_functions_match_method_outputs() {
        let d_method = DmaDescriptor::new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        let d_fn = dma_descriptor_new(src_addr(), dst_addr(), 8, DmaWidth::Word).unwrap();
        assert_eq!(d_method.source, d_fn.source);
        assert_eq!(d_method.dest, d_fn.dest);
        assert_eq!(d_method.length, d_fn.length);

        let mut chain: DmaChain<2> = DmaChain::new();
        dma_chain_append(&mut chain, d_fn).unwrap();
        assert_eq!(chain.len(), 1);

        let transfer = dma_start_transfer(chain);
        assert_eq!(transfer.descriptor_count(), 1);
    }

    // ---------- Total-bytes saturation ----------

    #[test]
    fn total_bytes_sums_across_chain() {
        let mut chain: DmaChain<4> = DmaChain::new();
        let lengths = [10usize, 200, 3000, 40000];
        for (i, &len) in lengths.iter().enumerate() {
            chain
                .append(
                    DmaDescriptor::new(src_addr() + i * 4, dst_addr() + i * 4, len, DmaWidth::Byte)
                        .unwrap(),
                )
                .unwrap();
        }
        let transfer = chain.start();
        let expected: u64 = lengths.iter().map(|&n| n as u64).sum();
        assert_eq!(transfer.total_bytes(), expected);
    }

    // ---------- Simulated DMA execution ----------
    //
    // The hardware would walk the chain itself. On the host we
    // simulate by reading `source`, writing `dest`, and chasing
    // `next` from Rust — exercising the same pointer layout the
    // engine would follow.

    #[test]
    fn simulated_engine_walks_chain_and_copies_bytes() {
        // Stack-local source/dest so we have writeable buffers.
        let src = [0xAAu8; 32];
        let mut dst = [0x00u8; 32];

        let mut chain: DmaChain<4> = DmaChain::new();
        // Three descriptors covering bytes 0..8, 8..16, 16..32.
        for (offset, len) in [(0usize, 8usize), (8, 8), (16, 16)] {
            let d = DmaDescriptor::new(
                unsafe { src.as_ptr().add(offset) } as usize,
                unsafe { dst.as_mut_ptr().add(offset) } as usize,
                len,
                DmaWidth::Byte,
            )
            .unwrap();
            chain.append(d).unwrap();
        }
        let transfer = chain.start();
        let head = transfer.head_ptr();

        // Walk the chain, mimicking what the hardware does.
        let mut node = head;
        while !node.is_null() {
            // SAFETY: each node in the chain is alive (owned by
            // `transfer`) and the source/dest addresses cover the
            // declared stack buffers.
            unsafe {
                let n = &*node;
                ptr::copy_nonoverlapping(
                    n.source as *const u8,
                    n.dest as *mut u8,
                    n.length as usize,
                );
                node = n.next;
            }
        }

        assert_eq!(&dst[..], &[0xAAu8; 32][..]);
    }
}
