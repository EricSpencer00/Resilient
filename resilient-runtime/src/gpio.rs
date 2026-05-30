//! RES-2593: GPIO hardware abstraction layer.
//!
//! Typestate-based wrapper around `core::ptr::{read,write}_volatile`
//! that lifts raw register pokes into safe, typed pin operations.
//! The typestate parameter (`Input` / `Output`) is enforced by the
//! Rust type system: `set_high` / `set_low` / `toggle` are only
//! available on `GpioPin<_, Output>`, `read` is only available on
//! `GpioPin<_, Input>`, and crossing between the two requires an
//! explicit `into_input` / `into_output` reconfiguration call.
//!
//! # Why typestate?
//!
//! Raw register pokes work but are easy to get wrong: writing to a
//! pin configured as input is a no-op the silicon silently swallows,
//! and reading an output pin is platform-dependent. Modeling the
//! configuration in the type system makes the wrong calls
//! non-existent rather than non-functional.
//!
//! # Why const-fn register addresses?
//!
//! The MCU's GPIO register block lives at a chip-defined base
//! address (e.g. STM32F4 GPIOA = `0x4002_0000`). Per-port offset
//! (`0x400`), per-pin shift inside the MODER / BSRR / IDR / ODR
//! registers, and the bit semantics of each register are all known
//! at compile time. Folding the address arithmetic into `const fn`s
//! lets the compiler turn `pin.set_high()` into a single 32-bit
//! store at a known address — the same code a hand-written driver
//! would emit.
//!
//! # Parameterising per chip
//!
//! Every chip lays out its GPIO block differently. The
//! [`GpioConfig`] trait carves out the contract: "given a port and
//! pin number, what is the address of each control register, and
//! how does the per-pin bit layout look?" Different chips implement
//! this trait; [`Stm32f4`] is the reference implementation covering
//! the demo board. Down-stream users with other parts (NXP, RP2040,
//! ESP32-C3) provide their own `GpioConfig`-implementing zero-sized
//! type and the rest of the API is identical.
//!
//! # `no_std`
//!
//! The module uses zero heap, no `std` types, no `format!`. Phantom
//! data carries the typestate parameter without runtime cost. The
//! few `unsafe` blocks all bottom out in `core::ptr::*_volatile`;
//! their soundness rests on [`GpioConfig`]'s safety contract that
//! the addresses returned by its const-fn methods are valid
//! memory-mapped peripheral addresses on the running hardware.
//!
//! # Mock backend for host-side tests
//!
//! Real MMIO addresses (e.g. `0x4002_0000`) are not writable on a
//! desktop machine — the OS will SIGSEGV the test runner. Tests
//! therefore use a private mock config that routes the same
//! address-computation arithmetic at an in-process buffer, letting
//! us exercise the typestate transitions and bit-twiddling on the
//! host. The cross-compile build of the crate against
//! `thumbv7em-none-eabihf` (`--no-default-features`) is the ground
//! truth for the no_std discipline; the host tests are ground truth
//! for the logic.

use core::marker::PhantomData;

/// Marker type: pin is configured as a digital input. Reads return
/// `bool`; calls to `set_high` / `set_low` / `toggle` do not
/// compile.
#[derive(Debug, Clone, Copy)]
pub struct Input;

/// Marker type: pin is configured as a digital output. Writes go
/// through `set_high` / `set_low` / `toggle`; calls to `read` do
/// not compile.
#[derive(Debug, Clone, Copy)]
pub struct Output;

/// Hardware ports. A chip may not implement all of them — the
/// [`GpioConfig`] trait's `port_base_addr` returns `None` for ports
/// the chip lacks, which the builder converts into a [`GpioError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Port {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
}

impl Port {
    /// Zero-based index, used for offset arithmetic. `A == 0`,
    /// `B == 1`, etc. Const-fn so callers can fold it.
    #[inline]
    pub const fn index(self) -> u32 {
        match self {
            Port::A => 0,
            Port::B => 1,
            Port::C => 2,
            Port::D => 3,
            Port::E => 4,
            Port::F => 5,
            Port::G => 6,
            Port::H => 7,
        }
    }
}

/// Errors the GPIO builders can return. All variants are
/// constructible without allocation — the runtime stays heap-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpioError {
    /// The pin number was outside `0..16`. STM32-family ports have
    /// 16 pins each; chips with wider ports can lift this in their
    /// own [`GpioConfig`] but the enum-of-`Port` API tops out at 16
    /// per port for V1.
    InvalidPin(u8),
    /// The selected port does not exist on this chip. Returned by
    /// [`GpioConfig::port_base_addr`] returning `None`.
    UnsupportedPort(Port),
}

/// Per-chip GPIO register layout. Implementors are zero-sized
/// configuration types: [`Stm32f4`] for the demo board, and
/// anything else users care to write for their own silicon (tests
/// in this module use a private mock).
///
/// # Safety
///
/// Implementations whose `port_base_addr` / register-offset
/// const-fns return real MMIO addresses are asserting that those
/// addresses are valid for volatile reads and writes on the running
/// hardware. Calling `set_high` etc. on a pin built against such a
/// config writes those addresses without further checking — the
/// trait IS the safety contract. The provided [`Stm32f4`] config is
/// only sound when the running silicon is actually an STM32F4 with
/// the GPIO clock enabled in RCC; mis-applying it to a different
/// MCU is undefined behaviour.
///
/// The trait is marked `unsafe` because impl authors are
/// responsible for upholding the contract; call sites that operate
/// the resulting pins are safe Rust.
pub unsafe trait GpioConfig {
    /// Base address of the named port's register block, or `None`
    /// if the chip does not have that port. The address is returned
    /// as `usize` (the host pointer width) so the same code paths
    /// work on 32-bit MCUs and 64-bit hosts.
    fn port_base_addr(port: Port) -> Option<usize>;

    /// Offset of the mode register (input/output/alternate/analog)
    /// inside the port block. The mode register is 32 bits with
    /// two bits per pin (so `MODER[pin*2 +: 2]` controls one pin).
    /// `Stm32f4` returns `0x00`; chips that lay the registers out
    /// differently override.
    fn moder_offset() -> usize;

    /// Offset of the output-data register. One bit per pin — the
    /// low 16 bits of a 32-bit word. Reading `ODR` tells you what
    /// you most recently wrote; writing it drives the pin.
    fn odr_offset() -> usize;

    /// Offset of the input-data register. One bit per pin — the
    /// low 16 bits of a 32-bit word. Reading `IDR` returns the
    /// live pin level (after the input synchroniser).
    fn idr_offset() -> usize;

    /// Offset of the bit-set/reset register. Writing 1 to bit `n`
    /// of the low half sets pin `n` high; writing 1 to bit `n+16`
    /// resets pin `n` low. The atomic set/reset means a single
    /// store cannot race with itself, which matters for
    /// interrupt-driven code that may share a port.
    fn bsrr_offset() -> usize;

    /// Two-bit mode value for "digital input". `Stm32f4` uses
    /// `0b00`.
    fn mode_input() -> u32;

    /// Two-bit mode value for "general-purpose digital output".
    /// `Stm32f4` uses `0b01`.
    fn mode_output() -> u32;
}

/// A configured GPIO pin. `CFG` is the chip-specific config type
/// (e.g. [`Stm32f4`]); `MODE` is the typestate marker ([`Input`] or
/// [`Output`]).
///
/// The struct is `Copy` because it is just `(Port, u8)` — moving a
/// pin handle would prevent natural patterns like passing the pin
/// to a function and continuing to use it. The hardware state is
/// the source of truth; the handle is a thin façade.
#[derive(Debug, Clone, Copy)]
pub struct GpioPin<CFG, MODE> {
    port: Port,
    pin: u8,
    _cfg: PhantomData<CFG>,
    _mode: PhantomData<MODE>,
}

impl<CFG: GpioConfig, MODE> GpioPin<CFG, MODE> {
    /// Port this pin lives on.
    #[inline]
    pub const fn port(&self) -> Port {
        self.port
    }

    /// Pin number within the port (0..16).
    #[inline]
    pub const fn pin(&self) -> u8 {
        self.pin
    }

    /// Validate `(port, pin)` against the chip config and return
    /// the per-port base address. Shared by both builders.
    #[inline]
    fn resolve(port: Port, pin: u8) -> Result<usize, GpioError> {
        if pin >= 16 {
            return Err(GpioError::InvalidPin(pin));
        }
        CFG::port_base_addr(port).ok_or(GpioError::UnsupportedPort(port))
    }
}

/// Build a pin configured as a digital output. Writes the MODER
/// register on construction, so the returned handle is ready for
/// immediate `set_high` / `set_low` / `toggle` calls.
///
/// # Errors
///
/// Returns [`GpioError::InvalidPin`] if `pin >= 16`, or
/// [`GpioError::UnsupportedPort`] if the chip config does not
/// implement that port.
#[inline]
pub fn gpio_output<CFG: GpioConfig>(
    port: Port,
    pin: u8,
) -> Result<GpioPin<CFG, Output>, GpioError> {
    let base = GpioPin::<CFG, Output>::resolve(port, pin)?;
    let moder_addr = base + CFG::moder_offset();
    let shift = (pin as u32) * 2;
    let mask = 0b11u32 << shift;
    let new_val = CFG::mode_output() << shift;
    // SAFETY: `base` came from `CFG::port_base_addr`, whose
    // returned addresses are valid by the trait's safety contract.
    // `moder_addr` is `base + moder_offset`, still inside the port
    // block. Read-modify-write of a 32-bit aligned MMIO register
    // is the canonical pattern for configuring one pin without
    // disturbing the other 15 in the same port.
    unsafe {
        let current = core::ptr::read_volatile(moder_addr as *const u32);
        core::ptr::write_volatile(moder_addr as *mut u32, (current & !mask) | new_val);
    }
    Ok(GpioPin {
        port,
        pin,
        _cfg: PhantomData,
        _mode: PhantomData,
    })
}

/// Build a pin configured as a digital input.
///
/// # Errors
///
/// See [`gpio_output`].
#[inline]
pub fn gpio_input<CFG: GpioConfig>(port: Port, pin: u8) -> Result<GpioPin<CFG, Input>, GpioError> {
    let base = GpioPin::<CFG, Input>::resolve(port, pin)?;
    let moder_addr = base + CFG::moder_offset();
    let shift = (pin as u32) * 2;
    let mask = 0b11u32 << shift;
    let new_val = CFG::mode_input() << shift;
    // SAFETY: see `gpio_output`.
    unsafe {
        let current = core::ptr::read_volatile(moder_addr as *const u32);
        core::ptr::write_volatile(moder_addr as *mut u32, (current & !mask) | new_val);
    }
    Ok(GpioPin {
        port,
        pin,
        _cfg: PhantomData,
        _mode: PhantomData,
    })
}

impl<CFG: GpioConfig> GpioPin<CFG, Output> {
    /// Drive the pin high. Uses the atomic BSRR write — a single
    /// store to bit `pin` of the low half. Will not race with
    /// other pin writes on the same port.
    #[inline]
    pub fn set_high(&self) {
        let base = CFG::port_base_addr(self.port).expect("pin built => port addr exists");
        let bsrr_addr = base + CFG::bsrr_offset();
        let bit = 1u32 << self.pin;
        // SAFETY: `bsrr_addr` is `base + bsrr_offset`, still inside
        // the port block validated at builder time. BSRR is a
        // write-only register — a single store sets exactly the
        // requested pin and ignores zero bits.
        unsafe {
            core::ptr::write_volatile(bsrr_addr as *mut u32, bit);
        }
    }

    /// Drive the pin low. Uses the upper half of BSRR (the reset
    /// bits) — bit `pin + 16` resets pin `pin`.
    #[inline]
    pub fn set_low(&self) {
        let base = CFG::port_base_addr(self.port).expect("pin built => port addr exists");
        let bsrr_addr = base + CFG::bsrr_offset();
        let bit = 1u32 << (self.pin as u32 + 16);
        // SAFETY: same reasoning as `set_high`. The upper-half
        // reset bits live in the same 32-bit BSRR word.
        unsafe {
            core::ptr::write_volatile(bsrr_addr as *mut u32, bit);
        }
    }

    /// Invert the pin level. Reads `ODR` (so we can see what we
    /// last drove), XORs the pin bit, writes it back. Not atomic
    /// against concurrent BSRR writes; if you need lock-free toggle
    /// you must serialise via a critical section.
    #[inline]
    pub fn toggle(&self) {
        let base = CFG::port_base_addr(self.port).expect("pin built => port addr exists");
        let odr_addr = base + CFG::odr_offset();
        let bit = 1u32 << self.pin;
        // SAFETY: `odr_addr` is `base + odr_offset`, still inside
        // the port block. Read-modify-write is the only portable
        // way to toggle a single pin; the documented race with
        // concurrent BSRR is the caller's responsibility per the
        // method's doc.
        unsafe {
            let current = core::ptr::read_volatile(odr_addr as *const u32);
            core::ptr::write_volatile(odr_addr as *mut u32, current ^ bit);
        }
    }

    /// Reconfigure this pin as an input, consuming the output
    /// handle. The pin level after the transition depends on
    /// whether you have a pull-up / pull-down configured — the
    /// `PUPDR` register is out of scope for V1 and is left at its
    /// reset value (no pull).
    #[inline]
    pub fn into_input(self) -> GpioPin<CFG, Input> {
        gpio_input::<CFG>(self.port, self.pin).expect("pin built => valid")
    }
}

impl<CFG: GpioConfig> GpioPin<CFG, Input> {
    /// Sample the pin and return `true` if high, `false` if low.
    /// Reads the IDR register, which the silicon synchronises to
    /// the bus clock — there is no debounce.
    #[inline]
    pub fn read(&self) -> bool {
        let base = CFG::port_base_addr(self.port).expect("pin built => port addr exists");
        let idr_addr = base + CFG::idr_offset();
        let bit = 1u32 << self.pin;
        // SAFETY: `idr_addr` is `base + idr_offset`, still inside
        // the port block. IDR is read-only; a volatile read returns
        // the live level.
        unsafe { (core::ptr::read_volatile(idr_addr as *const u32) & bit) != 0 }
    }

    /// Reconfigure this pin as an output, consuming the input
    /// handle. The initial output level is whatever the ODR
    /// currently holds — call `set_low` immediately after if you
    /// need a defined initial state.
    #[inline]
    pub fn into_output(self) -> GpioPin<CFG, Output> {
        gpio_output::<CFG>(self.port, self.pin).expect("pin built => valid")
    }
}

/// STM32F4 family GPIO config (`thumbv7em-none-eabihf`).
///
/// Base addresses come from RM0090 §2.3 / §8.5: GPIOA at
/// `0x4002_0000`, each subsequent port `+0x400`. The MODER / IDR /
/// ODR / BSRR offsets are 0x00, 0x10, 0x14, 0x18 per the same
/// reference.
///
/// **You must enable the GPIO clock before using any pin on this chip.**
/// Configuring a pin without first enabling the bus clock is a silent
/// no-op — the register reads back as 0 and writes are discarded.
///
/// Use the `rcc` module (RES-2638) to enable the clock in a typed,
/// no_std way:
///
/// ```rust,no_run
/// use resilient_runtime::rcc::{self, Peripheral, Stm32f4Rcc};
/// rcc::enable_peripheral::<Stm32f4Rcc>(Peripheral::GpioA).unwrap();
/// // Now it is safe to configure pins on port A.
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Stm32f4;

// SAFETY: Addresses are taken from STM32F4 reference manual RM0090
// §2.3 / §8.5. They are valid memory-mapped peripheral addresses
// on any STM32F4-family part; the trait's safety contract is
// upheld so long as this config is only applied to a part that is
// in fact an STM32F4 (or a binary-compatible alternative) with the
// relevant GPIO clock enabled.
unsafe impl GpioConfig for Stm32f4 {
    #[inline]
    fn port_base_addr(port: Port) -> Option<usize> {
        const GPIOA_BASE: usize = 0x4002_0000;
        const PORT_STRIDE: usize = 0x400;
        Some(GPIOA_BASE + (port.index() as usize) * PORT_STRIDE)
    }
    #[inline]
    fn moder_offset() -> usize {
        0x00
    }
    #[inline]
    fn odr_offset() -> usize {
        0x14
    }
    #[inline]
    fn idr_offset() -> usize {
        0x10
    }
    #[inline]
    fn bsrr_offset() -> usize {
        0x18
    }
    #[inline]
    fn mode_input() -> u32 {
        0b00
    }
    #[inline]
    fn mode_output() -> u32 {
        0b01
    }
}

// ---------------------------------------------------------------
// Tests
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! Host-side tests.
    //!
    //! The real STM32 addresses (`0x4002_0000`, ...) are not
    //! writable on a desktop, so tests drive a private mock config
    //! `MockGpio` that routes the same address arithmetic at a
    //! static u32 buffer. The cross-compile job
    //! (`cargo build --target thumbv7em-none-eabihf
    //! --no-default-features`) is the ground truth for the no_std
    //! discipline; these tests are ground truth for the logic.

    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    // 8 ports × 0x40 bytes (enough to cover MODER 0x00, IDR 0x10,
    // ODR 0x14, BSRR 0x18) = 0x200 bytes = 0x80 u32 cells.
    const MOCK_PORT_STRIDE_BYTES: usize = 0x40;
    const TOTAL_CELLS: usize = 8 * (MOCK_PORT_STRIDE_BYTES / 4);

    struct MockState {
        regs: [u32; TOTAL_CELLS],
        input_levels: [[bool; 16]; 8],
    }

    impl MockState {
        const fn new() -> Self {
            MockState {
                regs: [0; TOTAL_CELLS],
                input_levels: [[false; 16]; 8],
            }
        }
    }

    static STATE: OnceLock<Mutex<MockState>> = OnceLock::new();

    fn lock() -> MutexGuard<'static, MockState> {
        STATE
            .get_or_init(|| Mutex::new(MockState::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Zero out the mock state. Every test that uses the mock
    /// should call this first so tests are order-independent.
    fn mock_reset() {
        let mut g = lock();
        g.regs = [0; TOTAL_CELLS];
        g.input_levels = [[false; 16]; 8];
    }

    /// Drive the input level for the given port + pin. Subsequent
    /// `read()` calls on a `GpioPin<MockGpio, Input>` for that
    /// pin return this level.
    fn mock_set_input_level(port: Port, pin: u8, high: bool) {
        let mut g = lock();
        g.input_levels[port.index() as usize][pin as usize] = high;
        let port_idx = port.index() as usize;
        let mut input_word = 0u32;
        for (i, hi) in g.input_levels[port_idx].iter().enumerate() {
            if *hi {
                input_word |= 1 << i;
            }
        }
        let cell = ((port_idx * MOCK_PORT_STRIDE_BYTES) + MockGpio::idr_offset()) / 4;
        g.regs[cell] = input_word;
    }

    /// Mock config: routes register reads/writes into the static
    /// `STATE.regs` buffer above. Returns "base addresses" that
    /// are the addresses of cells inside that buffer.
    #[derive(Debug, Clone, Copy)]
    struct MockGpio;

    // SAFETY: The "addresses" returned by `port_base_addr` are
    // real host addresses pointing into the `STATE.regs` static
    // buffer, which is aligned u32 storage. Volatile reads and
    // writes of u32 at those addresses are safe; the mock owns
    // the buffer and serialises access through the OnceLock +
    // Mutex.
    unsafe impl GpioConfig for MockGpio {
        fn port_base_addr(port: Port) -> Option<usize> {
            let g = lock();
            let cell = (port.index() as usize) * (MOCK_PORT_STRIDE_BYTES / 4);
            Some(g.regs.as_ptr() as usize + cell * 4)
        }
        fn moder_offset() -> usize {
            0x00
        }
        fn odr_offset() -> usize {
            0x14
        }
        fn idr_offset() -> usize {
            0x10
        }
        fn bsrr_offset() -> usize {
            0x18
        }
        fn mode_input() -> u32 {
            0b00
        }
        fn mode_output() -> u32 {
            0b01
        }
    }

    /// Read a u32 register from the mock buffer using the same
    /// address arithmetic the production code uses. Used by tests
    /// to verify that what the production code wrote is what we
    /// observe.
    fn read_mock_reg(port: Port, offset: usize) -> u32 {
        let base = MockGpio::port_base_addr(port).unwrap();
        let addr = base + offset;
        // SAFETY: aligned u32 mock cell; test-only.
        unsafe { core::ptr::read_volatile(addr as *const u32) }
    }

    // ---------- validation ----------

    #[test]
    fn invalid_pin_errors() {
        mock_reset();
        let err = gpio_output::<MockGpio>(Port::A, 16).unwrap_err();
        assert_eq!(err, GpioError::InvalidPin(16));
        let err = gpio_input::<MockGpio>(Port::A, 99).unwrap_err();
        assert_eq!(err, GpioError::InvalidPin(99));
    }

    #[test]
    fn port_index_is_stable() {
        // The MMIO maths relies on `Port::A == 0`. Lock that in.
        assert_eq!(Port::A.index(), 0);
        assert_eq!(Port::B.index(), 1);
        assert_eq!(Port::H.index(), 7);
    }

    // ---------- output writes ----------

    #[test]
    fn output_set_high_writes_low_half_of_bsrr() {
        mock_reset();
        let pin = gpio_output::<MockGpio>(Port::B, 3).expect("valid pin");
        pin.set_high();
        let bsrr = read_mock_reg(Port::B, MockGpio::bsrr_offset());
        assert_eq!(
            bsrr,
            1 << 3,
            "BSRR after set_high should have only bit 3 set in the low half"
        );
    }

    #[test]
    fn output_set_low_writes_upper_half_of_bsrr() {
        mock_reset();
        let pin = gpio_output::<MockGpio>(Port::B, 3).expect("valid pin");
        pin.set_low();
        let bsrr = read_mock_reg(Port::B, MockGpio::bsrr_offset());
        assert_eq!(
            bsrr,
            1 << (3 + 16),
            "BSRR after set_low should have only bit 19 set (reset of pin 3)"
        );
    }

    #[test]
    fn output_toggle_inverts_the_pin_via_odr() {
        mock_reset();
        let pin = gpio_output::<MockGpio>(Port::C, 7).expect("valid pin");
        // Start: ODR.7 = 0 (default after reset).
        pin.toggle();
        let odr_after_one = read_mock_reg(Port::C, MockGpio::odr_offset());
        assert!(odr_after_one & (1 << 7) != 0);
        pin.toggle();
        let odr_after_two = read_mock_reg(Port::C, MockGpio::odr_offset());
        assert!(odr_after_two & (1 << 7) == 0);
    }

    // ---------- input reads ----------

    #[test]
    fn input_read_returns_driven_level() {
        mock_reset();
        let pin = gpio_input::<MockGpio>(Port::D, 0).expect("valid pin");
        assert!(!pin.read(), "default low");
        mock_set_input_level(Port::D, 0, true);
        assert!(pin.read(), "driven high");
        mock_set_input_level(Port::D, 0, false);
        assert!(!pin.read(), "driven low again");
    }

    #[test]
    fn input_read_isolates_pins_within_port() {
        // Driving pin 0 must not affect pin 1, and vice versa.
        mock_reset();
        let p0 = gpio_input::<MockGpio>(Port::E, 0).expect("valid pin");
        let p1 = gpio_input::<MockGpio>(Port::E, 1).expect("valid pin");
        mock_set_input_level(Port::E, 0, true);
        assert!(p0.read());
        assert!(!p1.read());
        mock_set_input_level(Port::E, 1, true);
        assert!(p0.read());
        assert!(p1.read());
    }

    // ---------- typestate transitions ----------

    #[test]
    fn typestate_transition_output_to_input() {
        mock_reset();
        let out = gpio_output::<MockGpio>(Port::A, 2).expect("valid pin");
        out.set_high();
        let inp: GpioPin<MockGpio, Input> = out.into_input();
        mock_set_input_level(Port::A, 2, false);
        assert!(!inp.read());
        mock_set_input_level(Port::A, 2, true);
        assert!(inp.read());
    }

    #[test]
    fn typestate_transition_input_to_output() {
        mock_reset();
        let inp = gpio_input::<MockGpio>(Port::F, 4).expect("valid pin");
        let out = inp.into_output();
        out.set_high();
        let bsrr = read_mock_reg(Port::F, MockGpio::bsrr_offset());
        assert!(bsrr & (1 << 4) != 0);
    }

    // ---------- accessors ----------

    #[test]
    fn pin_handles_carry_port_and_pin_accessors() {
        mock_reset();
        let pin = gpio_output::<MockGpio>(Port::G, 11).expect("valid pin");
        assert_eq!(pin.port(), Port::G);
        assert_eq!(pin.pin(), 11);
    }

    // ---------- MODER bit twiddling ----------

    #[test]
    fn moder_bits_are_updated_per_pin_without_disturbing_neighbours() {
        mock_reset();
        let _p3 = gpio_output::<MockGpio>(Port::A, 3).expect("valid pin");
        let _p7 = gpio_output::<MockGpio>(Port::A, 7).expect("valid pin");
        let moder = read_mock_reg(Port::A, MockGpio::moder_offset());
        // Pin 3 occupies bits 6..=7, pin 7 occupies bits 14..=15.
        // Output mode = 0b01.
        assert_eq!((moder >> 6) & 0b11, 0b01, "pin 3 MODER");
        assert_eq!((moder >> 14) & 0b11, 0b01, "pin 7 MODER");
    }

    #[test]
    fn moder_clears_back_to_input_from_output() {
        mock_reset();
        let out = gpio_output::<MockGpio>(Port::B, 5).expect("valid pin");
        let moder_out = read_mock_reg(Port::B, MockGpio::moder_offset());
        assert_eq!((moder_out >> 10) & 0b11, 0b01, "output mode");
        let _inp = out.into_input();
        let moder_in = read_mock_reg(Port::B, MockGpio::moder_offset());
        assert_eq!((moder_in >> 10) & 0b11, 0b00, "input mode");
    }

    // ---------- STM32F4 reference-manual addresses ----------

    #[test]
    fn stm32f4_port_addresses_match_reference_manual() {
        // RM0090 §2.3: GPIOA = 0x4002_0000, GPIOB = 0x4002_0400.
        assert_eq!(Stm32f4::port_base_addr(Port::A), Some(0x4002_0000));
        assert_eq!(Stm32f4::port_base_addr(Port::B), Some(0x4002_0400));
        assert_eq!(Stm32f4::port_base_addr(Port::H), Some(0x4002_1C00));
    }

    #[test]
    fn stm32f4_register_offsets_match_reference_manual() {
        // RM0090 §8.4: MODER=0x00, IDR=0x10, ODR=0x14, BSRR=0x18.
        assert_eq!(Stm32f4::moder_offset(), 0x00);
        assert_eq!(Stm32f4::idr_offset(), 0x10);
        assert_eq!(Stm32f4::odr_offset(), 0x14);
        assert_eq!(Stm32f4::bsrr_offset(), 0x18);
    }

    #[test]
    fn stm32f4_mode_bit_values_match_reference_manual() {
        assert_eq!(Stm32f4::mode_input(), 0b00);
        assert_eq!(Stm32f4::mode_output(), 0b01);
    }
}
