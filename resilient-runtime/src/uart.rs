//! RES-2597: UART serial communication abstraction.
//!
//! A `#![no_std]`-clean UART surface that lets user code wire any
//! concrete peripheral — a hardware UART, a USB-CDC virtual COM port,
//! a semihosting channel, or a host-side ring-buffer mock — behind
//! one trait, [`UartIo`], and then talk to it through a typed
//! [`UartHandle<U>`].
//!
//! # Why a trait, not a concrete type?
//!
//! Embedded UARTs are extremely heterogeneous: the F4 PAC, the F1
//! PAC, the nRF52 PAC, and the RP2040 PAC all expose different
//! register shapes. Rather than bake a specific PAC into the
//! runtime (which would force every binary to pull that PAC in),
//! the runtime exposes a tiny `byte-in / byte-out / availability`
//! trait. The binary supplies a `UartIo` adapter that wraps the
//! actual peripheral; the runtime composes blocking byte-stream
//! semantics on top.
//!
//! This matches the [`Sink`][crate::sink::Sink] pattern already in
//! the runtime: the runtime owns the protocol, the user owns the
//! transport.
//!
//! # Shape
//!
//! - [`UartConfig`] — baud rate + framing (`data_bits`, `parity`,
//!   `stop_bits`). Plain data, `Copy`, no behavior.
//! - [`Parity`], [`DataBits`], [`StopBits`] — enums tagging the
//!   framing knobs the typical embedded UART exposes.
//! - [`UartError`] — narrow error type returned by every fallible op
//!   (init/read/write). Variants distinguish transport faults
//!   (`Hardware`) from configuration mistakes (`Unsupported`,
//!   `InvalidConfig`) and end-of-stream conditions (`Closed`).
//! - [`UartIo`] — the user-implemented trait. Three methods:
//!   `try_write_byte`, `try_read_byte`, `available_bytes`. Each
//!   one is non-blocking; the runtime layer composes the blocking
//!   policy on top.
//! - [`uart_init`] — applies a [`UartConfig`] to a `UartIo` and
//!   returns a [`UartHandle`]. The handle is the typed entry
//!   point for the rest of the API.
//! - [`UartHandle::write`] / [`UartHandle::read`] — blocking,
//!   byte-stream IO. Both spin against the underlying `try_*`
//!   methods until the slice is drained / filled or an error
//!   short-circuits.
//! - [`UartHandle::write_byte`] / [`UartHandle::read_byte`] —
//!   convenience single-byte versions.
//! - [`UartHandle::available`] — forwards `available_bytes()`.
//!
//! # Blocking policy
//!
//! `write` and `read` are blocking — they busy-loop until the
//! transport reports it can accept or produce more bytes. This is
//! the right default for the typical embedded use case (debug
//! print, sensor polling, small inter-MCU protocols) and matches
//! the issue's acceptance criteria. A future ticket will add
//! DMA-backed variants; the trait surface here is deliberately
//! narrow so the DMA layer can sit alongside without disturbing
//! the blocking API.
//!
//! Implementers running on a real MCU should yield to the
//! peripheral (e.g. WFE on Cortex-M, or a critical-section /
//! `cortex_m::asm::nop()` spin) inside `try_*`; the runtime treats
//! `Ok(None)` as "try again later" and re-enters the trait
//! immediately. The runtime does not impose its own delay loop —
//! that's the transport's responsibility.
//!
//! # `no_std`
//!
//! Nothing in this module pulls in `alloc` or `std`. The mock UART
//! used by the unit tests lives behind `#[cfg(test)]` and uses
//! fixed-size in-memory ring buffers built from stack arrays — it
//! does not require `alloc` either, so the same test runs on host
//! and (in principle) on an embedded test harness without changes.

use core::cell::Cell;
use core::fmt;

// ---------- configuration types ----------

/// Number of data bits per UART frame.
///
/// 8-bit is overwhelmingly the embedded default; 7-bit appears in
/// ASCII-only protocols and some legacy serial links; 9-bit appears
/// in addressable multi-drop buses (some industrial / automotive
/// stacks). Anything else is exotic enough that we'd rather reject
/// it at config time than silently truncate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataBits {
    Seven,
    Eight,
    Nine,
}

/// Parity bit configuration.
///
/// `None` — no parity bit appended. Most common modern setting.
/// `Even` / `Odd` — classic error-detection scheme. Still seen in
/// older industrial buses and some Bluetooth-modem profiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// Number of stop bits per UART frame.
///
/// 1 stop bit is the modern default. 2 stop bits appear in very
/// slow links / legacy hardware. 1.5 stop bits is associated with
/// 5-bit data and is exotic enough that we leave it out — adding
/// it later is a non-breaking enum extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopBits {
    One,
    Two,
}

/// User-facing UART configuration. Plain data; the transport
/// adapter interprets it.
///
/// Construct with `UartConfig::new(baud)` and chain `.parity(...)`,
/// `.data_bits(...)`, `.stop_bits(...)` for non-default framing.
///
/// # Defaults
///
/// `UartConfig::new(baud)` gives 8N1 framing — eight data bits, no
/// parity, one stop bit. This is the de-facto serial default and
/// matches what `screen /dev/ttyUSB0 115200` would talk to without
/// any extra flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartConfig {
    /// Baud rate in bits per second (e.g. 115_200).
    pub baud: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
}

impl UartConfig {
    /// New config at `baud` baud, 8N1 framing.
    pub const fn new(baud: u32) -> Self {
        Self {
            baud,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
        }
    }

    /// Override data bits.
    pub const fn data_bits(mut self, data_bits: DataBits) -> Self {
        self.data_bits = data_bits;
        self
    }

    /// Override parity.
    pub const fn parity(mut self, parity: Parity) -> Self {
        self.parity = parity;
        self
    }

    /// Override stop bits.
    pub const fn stop_bits(mut self, stop_bits: StopBits) -> Self {
        self.stop_bits = stop_bits;
        self
    }
}

// ---------- errors ----------

/// Errors returned by the UART surface.
///
/// `Copy` so callers can pattern-match without moving the value,
/// and so it composes nicely with `?` in `no_std` contexts where
/// boxing isn't available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UartError {
    /// `UartConfig` asked for a framing combination the transport
    /// cannot represent (e.g. nine data bits on a peripheral that
    /// only does 7/8). The transport surfaces this from
    /// [`UartIo::configure`].
    Unsupported,
    /// `UartConfig` is internally inconsistent (e.g. baud == 0).
    /// Detected by `uart_init` before the transport sees it.
    InvalidConfig,
    /// The transport reported a hardware error (framing, overrun,
    /// parity, break). The payload is deliberately unit — concrete
    /// transports log their own richer diagnostics; the runtime
    /// surface keeps the error narrow.
    Hardware,
    /// The transport reports the link is gone (host disconnected
    /// USB-CDC, mock peer dropped, etc.). Both `read` and `write`
    /// surface this so callers can break out of the blocking spin.
    Closed,
}

// ---------- transport trait ----------

/// User-implemented byte-level UART transport.
///
/// Implementers wrap a concrete peripheral (or a mock) and expose
/// three operations. All three are **non-blocking**:
///
/// - `try_write_byte` returns `Ok(true)` when the TX FIFO accepted
///   the byte, `Ok(false)` when it didn't and the caller should
///   retry, `Err(_)` on transport failure.
/// - `try_read_byte` returns `Ok(Some(b))` when a byte was
///   available and consumed, `Ok(None)` when none was available,
///   `Err(_)` on transport failure.
/// - `available_bytes` returns the current count of bytes that
///   `try_read_byte` would succeed on. Best-effort — implementers
///   that can only report 0-or-many should return 0 / 1 to keep
///   the semantics monotone.
///
/// `configure` is called exactly once by [`uart_init`] before any
/// IO. Implementers apply the framing settings to the peripheral.
/// Returning `Err(UartError::Unsupported)` is the right response if
/// the requested `UartConfig` can't be expressed.
pub trait UartIo {
    /// Apply the framing settings to the peripheral. Called once
    /// from `uart_init`. Default no-op for transports that ignore
    /// configuration (e.g. test mocks).
    fn configure(&mut self, _config: &UartConfig) -> Result<(), UartError> {
        Ok(())
    }

    /// Attempt to push one byte into the TX path without blocking.
    /// `Ok(true)` = accepted; `Ok(false)` = TX full, try again.
    fn try_write_byte(&mut self, byte: u8) -> Result<bool, UartError>;

    /// Attempt to pull one byte out of the RX path without
    /// blocking. `Ok(Some(_))` = got a byte; `Ok(None)` = nothing
    /// available right now.
    fn try_read_byte(&mut self) -> Result<Option<u8>, UartError>;

    /// Best-effort count of bytes currently available to read.
    /// May undercount (return 0 when there's actually one waiting
    /// in the shift register) but must not overcount.
    fn available_bytes(&self) -> usize;
}

// ---------- handle ----------

/// Typed UART handle wrapping a configured transport.
///
/// Construct with [`uart_init`]; do not build manually. The handle
/// borrows the transport mutably for the lifetime of the handle —
/// that's what gives us static guarantees that only one piece of
/// code is reading/writing the UART at a time without needing a
/// runtime mutex on the embedded side.
pub struct UartHandle<'a, U: UartIo> {
    io: &'a mut U,
    config: UartConfig,
    // RES-2597: bookkeeping for diagnostic counters. Cells so
    // `available()` (immutable borrow) can still update them.
    bytes_written: Cell<usize>,
    bytes_read: Cell<usize>,
}

// Manual Debug — `&mut U` doesn't implement Debug for arbitrary U,
// so the derived impl wouldn't compile. We surface the bookkeeping
// state and the config, which is what callers debugging a handle
// actually want to see.
impl<U: UartIo> fmt::Debug for UartHandle<'_, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UartHandle")
            .field("config", &self.config)
            .field("bytes_written", &self.bytes_written.get())
            .field("bytes_read", &self.bytes_read.get())
            .finish()
    }
}

impl<'a, U: UartIo> UartHandle<'a, U> {
    /// Apply `config` to `io` and return a handle. Equivalent to
    /// calling [`uart_init`] — provided for callers that prefer
    /// method syntax. Note `uart_init` is the documented entry
    /// point.
    fn new(io: &'a mut U, config: UartConfig) -> Result<Self, UartError> {
        validate_config(&config)?;
        io.configure(&config)?;
        Ok(Self {
            io,
            config,
            bytes_written: Cell::new(0),
            bytes_read: Cell::new(0),
        })
    }

    /// Return the configuration this handle was initialized with.
    pub fn config(&self) -> &UartConfig {
        &self.config
    }

    /// Blocking write: spins until every byte in `bytes` has been
    /// pushed into the TX path, or a transport error short-circuits.
    /// Returns the number of bytes successfully written, which is
    /// `bytes.len()` on success.
    pub fn write(&mut self, bytes: &[u8]) -> Result<usize, UartError> {
        for &b in bytes.iter() {
            loop {
                if self.io.try_write_byte(b)? {
                    break;
                }
                // Transport says "TX full, retry". We re-enter
                // try_write_byte immediately — the transport is
                // responsible for any CPU-yield/WFE strategy.
            }
            self.bytes_written
                .set(self.bytes_written.get().saturating_add(1));
        }
        Ok(bytes.len())
    }

    /// Blocking write of a single byte. Convenience wrapper.
    pub fn write_byte(&mut self, byte: u8) -> Result<(), UartError> {
        self.write(&[byte]).map(|_| ())
    }

    /// Blocking read: spins until `buf` is full or an error
    /// short-circuits. Returns the number of bytes actually
    /// placed into `buf` — equal to `buf.len()` on success.
    ///
    /// To bound by `max_bytes < buf.len()`, pass
    /// `&mut buf[..max_bytes]`.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, UartError> {
        let mut filled = 0;
        while filled < buf.len() {
            if let Some(b) = self.io.try_read_byte()? {
                buf[filled] = b;
                filled += 1;
                self.bytes_read.set(self.bytes_read.get().saturating_add(1));
            }
            // None = spin until a byte shows up. Same yield
            // policy as write: transport's responsibility.
        }
        Ok(filled)
    }

    /// Blocking read of a single byte. Convenience wrapper.
    pub fn read_byte(&mut self) -> Result<u8, UartError> {
        let mut buf = [0u8; 1];
        self.read(&mut buf)?;
        Ok(buf[0])
    }

    /// Bytes currently available to read without blocking. Forwards
    /// to [`UartIo::available_bytes`].
    pub fn available(&self) -> usize {
        self.io.available_bytes()
    }

    /// Cumulative bytes successfully written through this handle.
    /// Saturating counter — useful for crude bandwidth-budget
    /// monitoring without pulling in `live_telemetry`.
    pub fn bytes_written(&self) -> usize {
        self.bytes_written.get()
    }

    /// Cumulative bytes successfully read through this handle.
    pub fn bytes_read(&self) -> usize {
        self.bytes_read.get()
    }
}

// ---------- top-level init function ----------

/// Initialize a UART transport with `config` and return a handle.
///
/// This is the canonical entry point — matches the issue's API:
///
/// ```ignore
/// use resilient_runtime::uart::{uart_init, UartConfig};
/// let mut uart = uart_init(&mut my_peripheral, UartConfig::new(115_200))?;
/// uart.write(b"Hello\r\n")?;
/// let mut buf = [0u8; 16];
/// uart.read(&mut buf)?;
/// ```
///
/// Validates `config` first (catches the trivial mistakes like
/// `baud == 0` before bothering the transport), then forwards
/// configuration to [`UartIo::configure`]. If the transport rejects
/// the framing as unsupported, `uart_init` propagates that error
/// unchanged.
pub fn uart_init<U: UartIo>(
    io: &mut U,
    config: UartConfig,
) -> Result<UartHandle<'_, U>, UartError> {
    UartHandle::new(io, config)
}

// ---------- standalone helpers (issue API parity) ----------
//
// The issue lists `uart_write` / `uart_read` / `uart_available` /
// `uart_read_byte` / `uart_write_byte` as named entry points. The
// idiomatic Rust API is `UartHandle::write` etc., but we also
// expose the bare-function spellings so the issue's API surface
// can be called verbatim and so the standalone-function spelling
// keeps working if/when the Resilient front-end binds these names
// directly.

/// Standalone spelling of [`UartHandle::write`].
pub fn uart_write<U: UartIo>(
    handle: &mut UartHandle<'_, U>,
    bytes: &[u8],
) -> Result<usize, UartError> {
    handle.write(bytes)
}

/// Standalone spelling of [`UartHandle::write_byte`].
pub fn uart_write_byte<U: UartIo>(
    handle: &mut UartHandle<'_, U>,
    byte: u8,
) -> Result<(), UartError> {
    handle.write_byte(byte)
}

/// Standalone spelling of [`UartHandle::read`].
///
/// Note: the issue's API is `uart_read(uart, max_bytes)`. In
/// `no_std` we can't return a fresh `Vec<u8>`, so the API takes a
/// buffer the caller owns. Pass `&mut buf[..max_bytes]` to cap
/// the read at `max_bytes`.
pub fn uart_read<U: UartIo>(
    handle: &mut UartHandle<'_, U>,
    buf: &mut [u8],
) -> Result<usize, UartError> {
    handle.read(buf)
}

/// Standalone spelling of [`UartHandle::read_byte`].
pub fn uart_read_byte<U: UartIo>(handle: &mut UartHandle<'_, U>) -> Result<u8, UartError> {
    handle.read_byte()
}

/// Standalone spelling of [`UartHandle::available`].
pub fn uart_available<U: UartIo>(handle: &UartHandle<'_, U>) -> usize {
    handle.available()
}

// ---------- internal validation ----------

fn validate_config(config: &UartConfig) -> Result<(), UartError> {
    if config.baud == 0 {
        return Err(UartError::InvalidConfig);
    }
    Ok(())
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    /// Host-side mock UART backed by two fixed-size ring buffers.
    /// Stack-allocated — no `alloc` dependency, so the same code
    /// would run on an embedded test target.
    struct MockUart {
        rx: RingBuf<256>,
        tx: RingBuf<256>,
        configured: Option<UartConfig>,
        // Test knobs.
        reject_config: bool,
        force_hardware_error_on_next_write: bool,
        force_hardware_error_on_next_read: bool,
        tx_pressure_cycles: u32, // make first N writes return false
    }

    impl MockUart {
        fn new() -> Self {
            Self {
                rx: RingBuf::new(),
                tx: RingBuf::new(),
                configured: None,
                reject_config: false,
                force_hardware_error_on_next_write: false,
                force_hardware_error_on_next_read: false,
                tx_pressure_cycles: 0,
            }
        }

        /// Seed the RX path so subsequent `read` calls drain it.
        fn feed_rx(&mut self, data: &[u8]) {
            for &b in data {
                self.rx.push(b).expect("rx capacity");
            }
        }

        /// Drain the TX path into a local buffer for assertion.
        /// Returns the count drained.
        fn drain_tx(&mut self, out: &mut [u8]) -> usize {
            let mut n = 0;
            while n < out.len() {
                match self.tx.pop() {
                    Some(b) => {
                        out[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            n
        }
    }

    impl UartIo for MockUart {
        fn configure(&mut self, config: &UartConfig) -> Result<(), UartError> {
            if self.reject_config {
                return Err(UartError::Unsupported);
            }
            self.configured = Some(*config);
            Ok(())
        }

        fn try_write_byte(&mut self, byte: u8) -> Result<bool, UartError> {
            if self.force_hardware_error_on_next_write {
                self.force_hardware_error_on_next_write = false;
                return Err(UartError::Hardware);
            }
            if self.tx_pressure_cycles > 0 {
                self.tx_pressure_cycles -= 1;
                return Ok(false);
            }
            match self.tx.push(byte) {
                Ok(()) => Ok(true),
                Err(()) => Ok(false),
            }
        }

        fn try_read_byte(&mut self) -> Result<Option<u8>, UartError> {
            if self.force_hardware_error_on_next_read {
                self.force_hardware_error_on_next_read = false;
                return Err(UartError::Hardware);
            }
            Ok(self.rx.pop())
        }

        fn available_bytes(&self) -> usize {
            self.rx.len()
        }
    }

    /// Tiny fixed-capacity ring buffer, stack-allocated.
    struct RingBuf<const N: usize> {
        data: [u8; N],
        head: usize,
        tail: usize,
        len: usize,
    }

    impl<const N: usize> RingBuf<N> {
        const fn new() -> Self {
            Self {
                data: [0; N],
                head: 0,
                tail: 0,
                len: 0,
            }
        }

        fn push(&mut self, b: u8) -> Result<(), ()> {
            if self.len == N {
                return Err(());
            }
            self.data[self.tail] = b;
            self.tail = (self.tail + 1) % N;
            self.len += 1;
            Ok(())
        }

        fn pop(&mut self) -> Option<u8> {
            if self.len == 0 {
                return None;
            }
            let b = self.data[self.head];
            self.head = (self.head + 1) % N;
            self.len -= 1;
            Some(b)
        }

        fn len(&self) -> usize {
            self.len
        }
    }

    // ---------- config builder ----------

    #[test]
    fn config_defaults_to_8n1() {
        let cfg = UartConfig::new(115_200);
        assert_eq!(cfg.baud, 115_200);
        assert_eq!(cfg.data_bits, DataBits::Eight);
        assert_eq!(cfg.parity, Parity::None);
        assert_eq!(cfg.stop_bits, StopBits::One);
    }

    #[test]
    fn config_builder_overrides() {
        let cfg = UartConfig::new(9600)
            .data_bits(DataBits::Seven)
            .parity(Parity::Even)
            .stop_bits(StopBits::Two);
        assert_eq!(cfg.baud, 9600);
        assert_eq!(cfg.data_bits, DataBits::Seven);
        assert_eq!(cfg.parity, Parity::Even);
        assert_eq!(cfg.stop_bits, StopBits::Two);
    }

    // ---------- init ----------

    #[test]
    fn init_applies_config_to_transport() {
        let mut mock = MockUart::new();
        let cfg = UartConfig::new(115_200).parity(Parity::Odd);
        {
            let handle = uart_init(&mut mock, cfg).unwrap();
            assert_eq!(handle.config().baud, 115_200);
            assert_eq!(handle.config().parity, Parity::Odd);
        }
        assert_eq!(mock.configured.unwrap().baud, 115_200);
        assert_eq!(mock.configured.unwrap().parity, Parity::Odd);
    }

    #[test]
    fn init_rejects_zero_baud_before_transport() {
        let mut mock = MockUart::new();
        let cfg = UartConfig::new(0);
        let err = uart_init(&mut mock, cfg).unwrap_err();
        assert_eq!(err, UartError::InvalidConfig);
        // Transport was never asked.
        assert!(mock.configured.is_none());
    }

    #[test]
    fn init_propagates_transport_unsupported() {
        let mut mock = MockUart::new();
        mock.reject_config = true;
        let cfg = UartConfig::new(115_200);
        let err = uart_init(&mut mock, cfg).unwrap_err();
        assert_eq!(err, UartError::Unsupported);
    }

    // ---------- write / read ----------

    #[test]
    fn write_pushes_bytes_into_tx() {
        let mut mock = MockUart::new();
        {
            let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
            let n = handle.write(b"hi").unwrap();
            assert_eq!(n, 2);
            assert_eq!(handle.bytes_written(), 2);
        }
        let mut out = [0u8; 8];
        let drained = mock.drain_tx(&mut out);
        assert_eq!(drained, 2);
        assert_eq!(&out[..drained], b"hi");
    }

    #[test]
    fn write_byte_round_trips() {
        let mut mock = MockUart::new();
        {
            let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
            uart_write_byte(&mut handle, 0x42).unwrap();
        }
        let mut out = [0u8; 1];
        let drained = mock.drain_tx(&mut out);
        assert_eq!(drained, 1);
        assert_eq!(out[0], 0x42);
    }

    #[test]
    fn read_drains_seeded_rx() {
        let mut mock = MockUart::new();
        mock.feed_rx(b"abcd");
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let mut buf = [0u8; 4];
        let n = uart_read(&mut handle, &mut buf).unwrap();
        assert_eq!(n, 4);
        assert_eq!(&buf, b"abcd");
        assert_eq!(handle.bytes_read(), 4);
    }

    #[test]
    fn read_byte_returns_first_byte() {
        let mut mock = MockUart::new();
        mock.feed_rx(b"X");
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let b = uart_read_byte(&mut handle).unwrap();
        assert_eq!(b, b'X');
    }

    #[test]
    fn available_reports_rx_depth() {
        let mut mock = MockUart::new();
        mock.feed_rx(b"abc");
        let handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        assert_eq!(uart_available(&handle), 3);
    }

    // ---------- blocking semantics ----------

    #[test]
    fn write_spins_through_tx_pressure() {
        let mut mock = MockUart::new();
        // First 3 try_write_byte calls return Ok(false); 4th
        // succeeds. write() must retry, not error.
        mock.tx_pressure_cycles = 3;
        {
            let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
            handle.write(b"!").unwrap();
        }
        let mut out = [0u8; 1];
        let drained = mock.drain_tx(&mut out);
        assert_eq!(drained, 1);
        assert_eq!(out[0], b'!');
    }

    #[test]
    fn write_surfaces_hardware_error() {
        let mut mock = MockUart::new();
        mock.force_hardware_error_on_next_write = true;
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let err = handle.write(b"x").unwrap_err();
        assert_eq!(err, UartError::Hardware);
    }

    #[test]
    fn read_surfaces_hardware_error() {
        let mut mock = MockUart::new();
        mock.force_hardware_error_on_next_read = true;
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let mut buf = [0u8; 1];
        let err = handle.read(&mut buf).unwrap_err();
        assert_eq!(err, UartError::Hardware);
    }

    // ---------- loopback test ----------

    /// Loopback adapter that wires `tx` straight to `rx`, so a
    /// single `UartHandle` can write a message and read it back.
    /// This is the canonical embedded acceptance test for any
    /// UART driver — short of a real wire-loopback rig, it's the
    /// closest we get to "real" round-trip exercise on host.
    struct LoopbackUart {
        // Single buffer used for both directions: bytes written
        // appear available to read.
        buf: RingBuf<256>,
        configured: Option<UartConfig>,
    }

    impl LoopbackUart {
        fn new() -> Self {
            Self {
                buf: RingBuf::new(),
                configured: None,
            }
        }
    }

    impl UartIo for LoopbackUart {
        fn configure(&mut self, config: &UartConfig) -> Result<(), UartError> {
            self.configured = Some(*config);
            Ok(())
        }

        fn try_write_byte(&mut self, byte: u8) -> Result<bool, UartError> {
            match self.buf.push(byte) {
                Ok(()) => Ok(true),
                Err(()) => Ok(false),
            }
        }

        fn try_read_byte(&mut self) -> Result<Option<u8>, UartError> {
            Ok(self.buf.pop())
        }

        fn available_bytes(&self) -> usize {
            self.buf.len()
        }
    }

    #[test]
    fn loopback_round_trips_message() {
        let mut mock = LoopbackUart::new();
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let payload = b"Hello, UART!\r\n";

        // Write the full payload.
        let written = handle.write(payload).unwrap();
        assert_eq!(written, payload.len());

        // Available should match what we just wrote.
        assert_eq!(handle.available(), payload.len());

        // Read it back.
        let mut rx = [0u8; 14];
        assert_eq!(rx.len(), payload.len());
        let read = handle.read(&mut rx).unwrap();
        assert_eq!(read, payload.len());
        assert_eq!(&rx, payload);

        // Counters report the totals.
        assert_eq!(handle.bytes_written(), payload.len());
        assert_eq!(handle.bytes_read(), payload.len());
        // Nothing left to read.
        assert_eq!(handle.available(), 0);
    }

    #[test]
    fn loopback_byte_by_byte() {
        let mut mock = LoopbackUart::new();
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        for b in 0u8..16 {
            handle.write_byte(b).unwrap();
            let got = handle.read_byte().unwrap();
            assert_eq!(got, b);
        }
    }

    #[test]
    fn empty_write_is_a_noop() {
        let mut mock = MockUart::new();
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let n = handle.write(&[]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(handle.bytes_written(), 0);
    }

    #[test]
    fn empty_read_buf_is_a_noop() {
        let mut mock = MockUart::new();
        let mut handle = uart_init(&mut mock, UartConfig::new(115_200)).unwrap();
        let mut buf: [u8; 0] = [];
        let n = handle.read(&mut buf).unwrap();
        assert_eq!(n, 0);
        assert_eq!(handle.bytes_read(), 0);
    }

    #[test]
    fn config_is_copy_so_callers_can_reuse() {
        let cfg = UartConfig::new(9600);
        let _cfg2 = cfg; // not a move
        let _cfg3 = cfg; // still usable
    }
}
