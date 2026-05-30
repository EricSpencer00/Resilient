//! RES-2598: Interrupt priority management for Cortex-M targets.
//!
//! Provides RAII critical sections, NVIC priority configuration,
//! and the `interrupt_disable` / `interrupt_enable` primitives
//! that `#[interrupt(priority = N)]` handlers rely on.
//!
//! # Priority model
//!
//! Cortex-M NVIC priorities are 8-bit values where **lower numeric
//! value = higher urgency**. This module presents a `Priority`
//! newtype that validates the value is in 0–15 (the top 4 bits of
//! the NVIC register, which is the subset guaranteed by the
//! architecture regardless of how many bits the chip implements).
//!
//! ```text
//! Priority(0)  — highest urgency (only exception: NMI / HardFault)
//! Priority(15) — lowest urgency (runs when no higher-priority ISR is pending)
//! ```
//!
//! # Critical sections
//!
//! `CriticalSection::enter()` atomically disables all maskable
//! interrupts (sets PRIMASK) and returns an RAII guard. On drop the
//! guard re-enables interrupts — but ONLY if they were enabled when
//! `enter()` was called (saving / restoring PRIMASK rather than
//! unconditionally re-enabling).
//!
//! ```rust
//! use resilient_runtime::interrupt::CriticalSection;
//! let _cs = CriticalSection::enter();
//! // interrupts disabled here
//! // interrupts restored on drop
//! ```
//!
//! # NVIC priority registers
//!
//! The Cortex-M NVIC exposes 240 8-bit priority registers starting
//! at `0xE000_E400`. `set_priority(irq, prio)` writes the top 4
//! bits of that register (the architecturally-meaningful bits).

#![allow(dead_code)]

/// Interrupt priority level on Cortex-M NVIC.
///
/// Valid range is 0–15 where 0 is the highest urgency.
/// The constructor validates the range and returns `None` for out-of-range values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority(u8);

impl Priority {
    /// Construct a `Priority` from a raw level (0–15).
    /// Returns `None` if `level > 15`.
    pub const fn new(level: u8) -> Option<Self> {
        if level <= 15 {
            Some(Priority(level))
        } else {
            None
        }
    }

    /// The raw 0–15 level.
    pub const fn level(self) -> u8 {
        self.0
    }

    /// Convert to the NVIC register byte (top 4 bits = priority,
    /// bottom 4 bits = 0 per architecture spec).
    pub const fn to_nvic_byte(self) -> u8 {
        self.0 << 4
    }

    pub const HIGHEST: Priority = Priority(0);
    pub const HIGH: Priority = Priority(4);
    pub const NORMAL: Priority = Priority(8);
    pub const LOW: Priority = Priority(12);
    pub const LOWEST: Priority = Priority(15);
}

/// RAII guard that disables interrupts on creation and restores
/// PRIMASK on drop.
///
/// Nesting is safe: the original PRIMASK is captured on `enter()`
/// and restored on `Drop`, so the outermost `enter()` controls
/// whether interrupts are ultimately re-enabled.
pub struct CriticalSection {
    /// PRIMASK value captured before we disabled interrupts.
    primask_saved: u32,
}

impl CriticalSection {
    /// Enter a critical section: disable maskable interrupts and
    /// capture the current PRIMASK so it can be restored on drop.
    #[inline]
    pub fn enter() -> Self {
        let primask_saved = read_primask();
        disable_interrupts();
        Self { primask_saved }
    }
}

impl Drop for CriticalSection {
    #[inline]
    fn drop(&mut self) {
        // Restore exactly what was there before — don't unconditionally enable.
        write_primask(self.primask_saved);
    }
}

/// Run `f` inside a critical section (interrupts disabled).
/// Returns the value that `f` returns.
///
/// This is the preferred form when the critical section fits in a closure —
/// it statically prevents escaping the `CriticalSection` guard.
#[inline]
pub fn with_critical_section<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _cs = CriticalSection::enter();
    f()
}

// ---------------------------------------------------------------------------
// NVIC priority register helpers
// ---------------------------------------------------------------------------

/// NVIC Interrupt Priority Registers base address on Cortex-M.
const NVIC_IPR_BASE: usize = 0xE000_E400;

/// Set the NVIC priority of IRQ number `irq` (0–239) to `priority`.
///
/// # Safety
/// Writes a volatile MMIO register. Must only be called from
/// privileged mode (Handler or Thread with full privileges).
#[inline]
pub unsafe fn set_priority(irq: u8, priority: Priority) {
    let addr = (NVIC_IPR_BASE + irq as usize) as *mut u8;
    // Safety: caller guarantees privileged mode; addr is a valid NVIC register.
    unsafe { addr.write_volatile(priority.to_nvic_byte()) }
}

/// Read back the priority of IRQ number `irq` from NVIC.
///
/// Returns the top-4-bit value (right-shifted by 4) as a `Priority`.
///
/// # Safety
/// Reads a volatile MMIO register; must be called from privileged mode.
#[inline]
pub unsafe fn get_priority(irq: u8) -> Priority {
    let addr = (NVIC_IPR_BASE + irq as usize) as *const u8;
    // Safety: caller guarantees privileged mode.
    let raw = unsafe { addr.read_volatile() };
    // The bottom 4 bits are unimplemented (read as 0); top 4 are priority.
    Priority(raw >> 4)
}

// ---------------------------------------------------------------------------
// PRIMASK helpers (target-specific)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "arm")]
#[inline]
fn read_primask() -> u32 {
    let primask: u32;
    // Safety: reading PRIMASK is always safe (no side effects).
    unsafe { core::arch::asm!("mrs {}, PRIMASK", out(reg) primask, options(nomem, nostack)) }
    primask
}

#[cfg(target_arch = "arm")]
#[inline]
fn write_primask(val: u32) {
    // Safety: restoring PRIMASK to a previously read value — no new capability granted.
    unsafe { core::arch::asm!("msr PRIMASK, {}", in(reg) val, options(nomem, nostack)) }
}

/// Disable all maskable interrupts (set PRIMASK = 1).
///
/// Prefer `CriticalSection::enter()` or `with_critical_section` for RAII safety.
#[cfg(target_arch = "arm")]
#[inline]
pub fn disable_interrupts() {
    // Safety: cpsid i is always safe to execute; it only affects the calling core.
    unsafe { core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags)) }
}

/// Enable maskable interrupts (clear PRIMASK).
///
/// Prefer `CriticalSection` drop for RAII safety.
#[cfg(target_arch = "arm")]
#[inline]
pub fn enable_interrupts() {
    // Safety: cpsie i enables interrupts — the caller must ensure no critical
    // section is still logically active.
    unsafe { core::arch::asm!("cpsie i", options(nomem, nostack, preserves_flags)) }
}

// Host/test stubs — PRIMASK doesn't exist on non-ARM.
#[cfg(not(target_arch = "arm"))]
fn read_primask() -> u32 {
    0
}
#[cfg(not(target_arch = "arm"))]
fn write_primask(_val: u32) {}
#[cfg(not(target_arch = "arm"))]
pub fn disable_interrupts() {}
#[cfg(not(target_arch = "arm"))]
pub fn enable_interrupts() {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_range() {
        assert!(Priority::new(0).is_some());
        assert!(Priority::new(15).is_some());
        assert!(Priority::new(16).is_none());
        assert!(Priority::new(255).is_none());
    }

    #[test]
    fn priority_to_nvic_byte() {
        assert_eq!(Priority::new(0).unwrap().to_nvic_byte(), 0x00);
        assert_eq!(Priority::new(1).unwrap().to_nvic_byte(), 0x10);
        assert_eq!(Priority::new(8).unwrap().to_nvic_byte(), 0x80);
        assert_eq!(Priority::new(15).unwrap().to_nvic_byte(), 0xF0);
    }

    #[test]
    fn priority_ordering() {
        assert!(Priority::HIGHEST < Priority::HIGH);
        assert!(Priority::HIGH < Priority::NORMAL);
        assert!(Priority::NORMAL < Priority::LOW);
        assert!(Priority::LOW < Priority::LOWEST);
    }

    #[test]
    fn critical_section_nesting_restores_state() {
        // On non-ARM we can only test the logic compiles and runs.
        {
            let _outer = CriticalSection::enter();
            {
                let _inner = CriticalSection::enter();
                // Both sections active here.
            }
            // Inner restored — outer still active.
        }
        // Outer restored — interrupts back to original state.
    }

    #[test]
    fn with_critical_section_returns_value() {
        let result = with_critical_section(|| 42u32);
        assert_eq!(result, 42);
    }
}
