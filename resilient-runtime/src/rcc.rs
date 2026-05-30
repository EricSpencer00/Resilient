//! RES-2638: RCC (Reset and Clock Control) HAL for embedded targets.
//!
//! Peripheral bus clocks must be enabled before the peripheral registers
//! can be read or written. On STM32F4, configuring a GPIO pin without
//! first enabling the AHB1 bus clock for that port is a silent no-op —
//! the MODER write is discarded and IDR reads back 0. This module provides
//! a typed, no_std API for enabling and disabling peripheral clocks so
//! users never have to poke `RCC->AHB1ENR` by hand.
//!
//! # Usage
//!
//! ```rust,no_run
//! use resilient_runtime::rcc::{self, Peripheral, Stm32f4Rcc};
//!
//! // Enable GPIOA clock before configuring any pin on port A.
//! rcc::enable_peripheral::<Stm32f4Rcc>(Peripheral::GpioA).unwrap();
//! ```
//!
//! # Design
//!
//! * [`Peripheral`] — enum of supported peripheral IDs. Variants map to
//!   specific clock-enable bits in the MCU's RCC register block.
//! * [`RccConfig`] — unsafe trait that maps a `Peripheral` to its
//!   enable-register address and bit position. Chip-specific structs
//!   implement this trait. [`Stm32f4Rcc`] is the reference implementation
//!   (STM32F4 RM0090 §7.3, AHB1ENR at `0x4002_3830`).
//! * [`enable_peripheral`], [`disable_peripheral`], [`is_enabled`] —
//!   safe wrappers around the `volatile_read`/`volatile_write` sequence.
//!
//! # `no_std`
//!
//! Zero heap, no `std` types. All unsafe is contained in the two
//! volatile-pointer operations and justified by the SAFETY comment on
//! [`RccConfig`].

// ---------------------------------------------------------------------------
// Peripheral enum
// ---------------------------------------------------------------------------

/// Peripheral identifiers understood by the RCC HAL.
///
/// Add variants for I2C, SPI, UART, TIM etc. here once their clock-enable
/// addresses are confirmed in the target's reference manual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peripheral {
    GpioA,
    GpioB,
    GpioC,
    GpioD,
    GpioE,
    GpioF,
    GpioG,
    GpioH,
}

// ---------------------------------------------------------------------------
// RccConfig trait
// ---------------------------------------------------------------------------

/// # Safety
///
/// `enable_register_addr(p)` must return the address of a 32-bit
/// read-write register whose bit at position `enable_bit(p)` is the
/// clock-enable flag for peripheral `p`. The address must be a valid,
/// memory-mapped peripheral address on the hardware running the code —
/// `enable_peripheral` writes directly to that address via
/// `core::ptr::write_volatile`. Returning an invalid address causes
/// undefined behaviour. The two methods must agree: they will always be
/// called together.
pub unsafe trait RccConfig {
    /// Address of the 32-bit clock-enable register for `peripheral`.
    /// Returns `None` if the peripheral is not supported on this chip.
    fn enable_register_addr(peripheral: Peripheral) -> Option<usize>;

    /// Bit index (0-based) within the clock-enable register for `peripheral`.
    /// Returns `None` if the peripheral is not supported on this chip.
    fn enable_bit(peripheral: Peripheral) -> Option<u32>;
}

// ---------------------------------------------------------------------------
// STM32F4 reference implementation
// ---------------------------------------------------------------------------

/// STM32F4-family RCC configuration.
///
/// Implements the eight GPIO port clock-enables via the AHB1 peripheral
/// clock enable register (`RCC_AHB1ENR`) at `0x4002_3830`. Bit assignments
/// follow STM32F4 reference manual RM0090 §7.3:
///
/// | Bit | GPIOAEN | GPIOBEN | GPIOCEN | … | GPIOHEN |
/// |-----|---------|---------|---------|---|---------|
/// | 0   | ✓       |         |         |   |         |
/// | …   |         | …       | …       |   |         |
/// | 7   |         |         |         |   | ✓       |
///
/// Use this config for any STM32F4xx variant. Verify the AHB1ENR address
/// against your specific part's reference manual if you are porting to a
/// different STM32 family — the F1 and F7 families lay out RCC differently.
#[derive(Debug, Clone, Copy)]
pub struct Stm32f4Rcc;

unsafe impl RccConfig for Stm32f4Rcc {
    #[inline]
    fn enable_register_addr(peripheral: Peripheral) -> Option<usize> {
        // STM32F4 RM0090 §7.3 — RCC_AHB1ENR at offset 0x30 from RCC base.
        // RCC base: 0x4002_3800.
        const RCC_AHB1ENR: usize = 0x4002_3830;
        match peripheral {
            Peripheral::GpioA
            | Peripheral::GpioB
            | Peripheral::GpioC
            | Peripheral::GpioD
            | Peripheral::GpioE
            | Peripheral::GpioF
            | Peripheral::GpioG
            | Peripheral::GpioH => Some(RCC_AHB1ENR),
        }
    }

    #[inline]
    fn enable_bit(peripheral: Peripheral) -> Option<u32> {
        // RM0090 §7.3.3: GPIOxEN is at bit N where GPIOA=0, GPIOB=1, …, GPIOH=7.
        let bit = match peripheral {
            Peripheral::GpioA => 0,
            Peripheral::GpioB => 1,
            Peripheral::GpioC => 2,
            Peripheral::GpioD => 3,
            Peripheral::GpioE => 4,
            Peripheral::GpioF => 5,
            Peripheral::GpioG => 6,
            Peripheral::GpioH => 7,
        };
        Some(bit)
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors returned by RCC HAL operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RccError {
    /// The peripheral is not supported by the configured chip.
    UnsupportedPeripheral,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Enable the bus clock for `peripheral`.
///
/// Performs a read-modify-write on the appropriate clock-enable register,
/// setting the enable bit for `peripheral`. Safe to call multiple times —
/// the bit-set is idempotent.
///
/// Returns `Err(RccError::UnsupportedPeripheral)` if the `CFG` implementation
/// does not support this peripheral.
///
/// # Safety (of the implementation)
///
/// All unsafe is encapsulated here. The two volatile operations — read and
/// write — are justified by [`RccConfig`]'s safety contract: the address
/// returned by [`RccConfig::enable_register_addr`] is a valid, 32-bit
/// read-write memory-mapped register.
pub fn enable_peripheral<CFG: RccConfig>(peripheral: Peripheral) -> Result<(), RccError> {
    let addr = CFG::enable_register_addr(peripheral).ok_or(RccError::UnsupportedPeripheral)?;
    let bit = CFG::enable_bit(peripheral).ok_or(RccError::UnsupportedPeripheral)?;
    // SAFETY: addr is a valid 32-bit MMIO register per RccConfig's contract.
    unsafe {
        let ptr = addr as *mut u32;
        let current = core::ptr::read_volatile(ptr);
        core::ptr::write_volatile(ptr, current | (1u32 << bit));
    }
    Ok(())
}

/// Disable the bus clock for `peripheral`.
///
/// Clears the enable bit in the clock-enable register. After this call
/// the peripheral's registers are inaccessible; writing to them is a
/// silent no-op and reads return undefined values.
pub fn disable_peripheral<CFG: RccConfig>(peripheral: Peripheral) -> Result<(), RccError> {
    let addr = CFG::enable_register_addr(peripheral).ok_or(RccError::UnsupportedPeripheral)?;
    let bit = CFG::enable_bit(peripheral).ok_or(RccError::UnsupportedPeripheral)?;
    // SAFETY: same as enable_peripheral.
    unsafe {
        let ptr = addr as *mut u32;
        let current = core::ptr::read_volatile(ptr);
        core::ptr::write_volatile(ptr, current & !(1u32 << bit));
    }
    Ok(())
}

/// Returns `true` if the bus clock for `peripheral` is currently enabled.
///
/// Returns `false` if the peripheral is unsupported or the enable bit is clear.
pub fn is_enabled<CFG: RccConfig>(peripheral: Peripheral) -> bool {
    let (Some(addr), Some(bit)) = (
        CFG::enable_register_addr(peripheral),
        CFG::enable_bit(peripheral),
    ) else {
        return false;
    };
    // SAFETY: same as enable_peripheral.
    unsafe {
        let ptr = addr as *const u32;
        (core::ptr::read_volatile(ptr) >> bit) & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    // A mock register for host-side testing. The real MMIO register is at
    // a fixed physical address; the mock routes address arithmetic into a
    // static atomic so tests can verify the read-modify-write logic without
    // mapping real hardware.
    static MOCK_AHB1ENR: AtomicU32 = AtomicU32::new(0);

    /// Mock RCC config that routes AHB1ENR accesses to `MOCK_AHB1ENR`.
    struct MockRcc;

    unsafe impl RccConfig for MockRcc {
        fn enable_register_addr(_peripheral: Peripheral) -> Option<usize> {
            Some(MOCK_AHB1ENR.as_ptr() as usize)
        }

        fn enable_bit(peripheral: Peripheral) -> Option<u32> {
            // Use the same bit layout as Stm32f4Rcc so tests exercise the
            // actual bit positions.
            let bit = match peripheral {
                Peripheral::GpioA => 0,
                Peripheral::GpioB => 1,
                Peripheral::GpioC => 2,
                Peripheral::GpioD => 3,
                Peripheral::GpioE => 4,
                Peripheral::GpioF => 5,
                Peripheral::GpioG => 6,
                Peripheral::GpioH => 7,
            };
            Some(bit)
        }
    }

    fn reset_mock() {
        MOCK_AHB1ENR.store(0, Ordering::SeqCst);
    }

    #[test]
    fn enable_sets_bit() {
        reset_mock();
        enable_peripheral::<MockRcc>(Peripheral::GpioA).unwrap();
        assert_eq!(
            MOCK_AHB1ENR.load(Ordering::SeqCst) & 0x1,
            1,
            "GPIOAEN bit should be set"
        );
    }

    #[test]
    fn enable_is_idempotent() {
        reset_mock();
        enable_peripheral::<MockRcc>(Peripheral::GpioB).unwrap();
        enable_peripheral::<MockRcc>(Peripheral::GpioB).unwrap();
        let val = MOCK_AHB1ENR.load(Ordering::SeqCst);
        assert_eq!(val, 0b10, "double-enable must not corrupt register");
    }

    #[test]
    fn disable_clears_bit() {
        reset_mock();
        enable_peripheral::<MockRcc>(Peripheral::GpioC).unwrap();
        disable_peripheral::<MockRcc>(Peripheral::GpioC).unwrap();
        let val = MOCK_AHB1ENR.load(Ordering::SeqCst);
        assert_eq!(val & 0b100, 0, "GPIOCEN bit should be cleared");
    }

    #[test]
    fn is_enabled_reflects_state() {
        reset_mock();
        assert!(!is_enabled::<MockRcc>(Peripheral::GpioD));
        enable_peripheral::<MockRcc>(Peripheral::GpioD).unwrap();
        assert!(is_enabled::<MockRcc>(Peripheral::GpioD));
        disable_peripheral::<MockRcc>(Peripheral::GpioD).unwrap();
        assert!(!is_enabled::<MockRcc>(Peripheral::GpioD));
    }

    #[test]
    fn enable_multiple_ports_independent() {
        reset_mock();
        enable_peripheral::<MockRcc>(Peripheral::GpioA).unwrap();
        enable_peripheral::<MockRcc>(Peripheral::GpioE).unwrap();
        let val = MOCK_AHB1ENR.load(Ordering::SeqCst);
        // GPIOAEN (bit 0) and GPIOEEN (bit 4) should both be set.
        assert_eq!(
            val & 0b0001_0001,
            0b0001_0001,
            "enabling A and E must set both bits independently"
        );
        // Disabling one must not affect the other.
        disable_peripheral::<MockRcc>(Peripheral::GpioA).unwrap();
        let val2 = MOCK_AHB1ENR.load(Ordering::SeqCst);
        assert_eq!(
            val2 & 0b0001_0001,
            0b0001_0000,
            "disabling A must leave E enabled"
        );
    }

    #[test]
    fn stm32f4_gpio_enable_register_is_ahb1enr() {
        assert_eq!(
            Stm32f4Rcc::enable_register_addr(Peripheral::GpioA),
            Some(0x4002_3830),
            "AHB1ENR address must match RM0090 §7.3"
        );
        assert_eq!(
            Stm32f4Rcc::enable_register_addr(Peripheral::GpioH),
            Some(0x4002_3830),
            "All GPIO ports share AHB1ENR on STM32F4"
        );
    }

    #[test]
    fn stm32f4_gpio_enable_bits() {
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioA), Some(0));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioB), Some(1));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioC), Some(2));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioD), Some(3));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioE), Some(4));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioF), Some(5));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioG), Some(6));
        assert_eq!(Stm32f4Rcc::enable_bit(Peripheral::GpioH), Some(7));
    }

    #[test]
    fn integration_enable_then_is_enabled() {
        reset_mock();
        enable_peripheral::<MockRcc>(Peripheral::GpioA).unwrap();
        assert!(
            is_enabled::<MockRcc>(Peripheral::GpioA),
            "is_enabled must return true after enable_peripheral"
        );
    }
}
