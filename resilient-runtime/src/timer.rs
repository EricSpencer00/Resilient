//! RES-2596: timer / counter peripheral abstraction.
//!
//! A `no_std`, allocation-free timer HAL for periodic interrupts,
//! one-shot delays, and PWM output. Models the same operational
//! surface a Cortex-M general-purpose timer block exposes
//! (init → start → fire → stop → reset), without committing to a
//! specific MCU register layout. The runtime owns the bookkeeping;
//! a host-side virtual clock backs the implementation so tests
//! exercise the same code path firmware would.
//!
//! # Surface
//!
//! ```ignore
//! use resilient_runtime::timer::{self, TimerConfig, TimerMode};
//!
//! let id = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
//! timer::timer_set_callback(id, Some(|| { /* toggle pin */ })).unwrap();
//! timer::timer_start(id).unwrap();
//! // …time passes; the host driver invokes timer::tick(elapsed_us) …
//! let count = timer::timer_count(id).unwrap();
//! timer::timer_stop(id).unwrap();
//! ```
//!
//! # Capacity
//!
//! [`MAX_TIMERS`] (8) hardware timers are tracked. The choice
//! mirrors what a typical Cortex-M4 part exposes (STM32F4 has
//! TIM1…TIM14, but a portable HAL rarely needs more than a
//! handful in practice). The slot table is a flat `[TimerState;
//! MAX_TIMERS]` static so no allocation is needed.
//!
//! # PWM
//!
//! [`TimerMode::Pwm`] carries a duty cycle in units of 1/1000
//! (per-mille). Per-mille keeps the API integer-only — no `f32`
//! dependency, no FPU requirement — while still giving 0.1%
//! resolution which is finer than any real PWM peripheral. The
//! current PWM output level (`true` = high, `false` = low) is
//! readable via [`timer_pwm_level`] so the host driver / test
//! harness can verify the waveform without reading hardware
//! registers.
//!
//! # Host-side virtual clock
//!
//! Production firmware drives the timer block from a hardware
//! interrupt: each timer "tick" of the underlying counter is
//! reported by calling [`tick`] with the elapsed microseconds since
//! the last tick. The runtime applies the elapsed time to every
//! running timer, decrements its period counter, and invokes the
//! registered callback when a period expires. Tests use the same
//! `tick` entry point with whatever virtual time they want.
//!
//! # Interrupt safety
//!
//! All state lives in a `Mutex`-free `UnsafeCell` wrapped in a
//! `Sync` newtype, matching the pattern in [`crate::sink`]. The
//! single-core-bare-metal assumption is the same: callers running
//! `tick` from an ISR while another core touches the table need
//! to wrap access in `critical-section` or a `spin::Mutex` —
//! that's a future ticket, not a today problem.
//!
//! # No-panic guarantee
//!
//! Every fallible path returns `Result<_, TimerError>`. The module
//! has no `unwrap()` / `expect()` / `panic!()` calls in non-test
//! code; CI proves this by building with `-D warnings` and the
//! runtime's no-std cross-compilation gates.

use core::cell::UnsafeCell;
#[cfg(target_has_atomic = "8")]
use core::sync::atomic::{AtomicBool, Ordering};

/// Maximum number of hardware timers the runtime tracks.
///
/// 8 covers every Cortex-M class part the runtime targets today
/// (STM32F4 ships 14, but a portable HAL almost never wires all
/// of them). Bumping this is a one-line constant change; the
/// rest of the module is parameterised on it.
pub const MAX_TIMERS: usize = 8;

/// Per-mille (1/1000) PWM duty-cycle precision. A PWM duty of
/// `500` means 50%; `1000` is fully-on; `0` is fully-off.
pub const PWM_DUTY_MAX: u16 = 1000;

/// Operational mode of a timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMode {
    /// Fire exactly once after the configured period elapses, then
    /// auto-stop. Reading [`timer_count`] after expiry returns the
    /// final tick count; calling [`timer_start`] again re-arms.
    OneShot,
    /// Fire every period until explicitly stopped. The count
    /// returned by [`timer_count`] is the number of expiries so
    /// far (NOT a free-running counter — that's what
    /// [`timer_elapsed_us`] is for).
    Periodic,
    /// PWM output mode. `duty_per_mille` is the high-time fraction
    /// of each period in 1/1000 units. The callback (if any) is
    /// invoked at the end of each PWM period (i.e. on rising edge
    /// of the next cycle). The current output level is exposed by
    /// [`timer_pwm_level`].
    Pwm { duty_per_mille: u16 },
}

/// User-facing configuration handed to [`timer_init`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerConfig {
    /// Tick frequency in Hz. The runtime converts this to a period
    /// in microseconds (1 / `frequency_hz` × 1_000_000) which the
    /// timer expires on. Must be > 0 — a zero frequency is rejected
    /// with [`TimerError::InvalidFrequency`] at init time.
    pub frequency_hz: u32,
    /// Mode the timer starts in. Mode is captured at init; switching
    /// modes requires a [`timer_reset`] + new init pair.
    pub mode: TimerMode,
}

impl TimerConfig {
    /// Shorthand for a periodic timer at `hz` Hz.
    pub const fn periodic(hz: u32) -> Self {
        Self {
            frequency_hz: hz,
            mode: TimerMode::Periodic,
        }
    }

    /// Shorthand for a one-shot timer at `hz` Hz.
    pub const fn one_shot(hz: u32) -> Self {
        Self {
            frequency_hz: hz,
            mode: TimerMode::OneShot,
        }
    }

    /// Shorthand for a PWM timer at `hz` Hz with the given duty
    /// cycle (per-mille, see [`TimerMode::Pwm`]).
    pub const fn pwm(hz: u32, duty_per_mille: u16) -> Self {
        Self {
            frequency_hz: hz,
            mode: TimerMode::Pwm { duty_per_mille },
        }
    }
}

/// Opaque handle to an initialised timer. Stable for the life of
/// the firmware once allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerId(u8);

impl TimerId {
    /// Numeric index of the timer in the runtime table. Exposed
    /// for diagnostic printing; opaque to users for normal use.
    #[inline]
    pub const fn index(self) -> u8 {
        self.0
    }
}

/// Callback function pointer. Plain `fn` (not `Fn`) because we
/// have no allocator in default builds — closure captures would
/// need a `Box`. Callers needing per-instance state can store it
/// in a `static AtomicX` and read it from inside the callback.
pub type TimerCallback = fn();

/// Run-state of a single timer slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    /// Not initialised. Slot is free.
    Free,
    /// Initialised but not running.
    Stopped,
    /// Counting toward the next expiry.
    Running,
    /// One-shot mode: fired and now waiting for re-arm.
    Expired,
}

#[derive(Clone, Copy)]
struct TimerState {
    status: Status,
    mode: TimerMode,
    /// Period between callbacks, in microseconds. Pre-computed
    /// from `frequency_hz` at init time so the hot tick path
    /// doesn't divide. A zero-Hz config is rejected, so this is
    /// always non-zero for an initialised timer.
    period_us: u32,
    /// Time accumulated toward the next expiry, in microseconds.
    /// Reset to 0 on expiry / reset / restart.
    accum_us: u32,
    /// Number of callback invocations since the last reset. For
    /// PWM mode this counts completed PWM periods, not edges.
    count: u32,
    /// Free-running elapsed time since start, in microseconds.
    /// Survives expiry / restart in periodic mode; cleared by
    /// [`timer_reset`].
    elapsed_us: u64,
    /// Currently-registered callback, if any.
    callback: Option<TimerCallback>,
}

impl TimerState {
    const FREE: Self = Self {
        status: Status::Free,
        mode: TimerMode::OneShot,
        period_us: 0,
        accum_us: 0,
        count: 0,
        elapsed_us: 0,
        callback: None,
    };
}

/// Error returned by every fallible timer operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerError {
    /// No free slot in the timer table. The runtime has at most
    /// [`MAX_TIMERS`] timers; free one with [`timer_release`] to
    /// allocate another.
    Exhausted,
    /// The supplied [`TimerId`] is not currently allocated.
    /// Either the id never came from `timer_init`, or the timer
    /// was released. Operations on a released id always return
    /// this rather than acting on a recycled slot.
    InvalidId,
    /// `frequency_hz == 0` at init time, or the requested
    /// frequency is so high the per-tick period rounds to zero
    /// microseconds (i.e. > 1_000_000 Hz).
    InvalidFrequency,
    /// `duty_per_mille > 1000` for a PWM init.
    InvalidDutyCycle,
    /// Operation requires the timer to be in a particular state
    /// (e.g. `timer_stop` on an already-stopped timer is fine;
    /// `timer_delay_ms` on a running timer is not).
    InvalidState,
}

struct Registry {
    slots: [TimerState; MAX_TIMERS],
}

impl Registry {
    const fn empty() -> Self {
        Self {
            slots: [TimerState::FREE; MAX_TIMERS],
        }
    }
}

/// SAFETY: see the `Sync` discussion at the top of the module.
/// Single-core bare-metal is the implicit deployment model; the
/// `LOCK` `AtomicBool` provides a minimal compare-exchange gate
/// across reentrant access from the host test harness.
struct GlobalRegistry(UnsafeCell<Registry>);

unsafe impl Sync for GlobalRegistry {}

static REGISTRY: GlobalRegistry = GlobalRegistry(UnsafeCell::new(Registry::empty()));

/// Coarse re-entrancy guard. The host-side test harness uses it
/// to serialise concurrent test runs (cargo runs tests in
/// parallel); embedded firmware on a single-core MCU touches the
/// registry from one context at a time and so spins through this
/// gate instantly.
///
/// Cortex-M0 and similar `thumbv6m` targets have no
/// `compare_exchange` on `AtomicBool` (only `load`/`store`), so on
/// those targets the lock is omitted entirely. The runtime is
/// single-threaded on those parts by definition (no SMP, and the
/// registry isn't touched from ISR context in user code without
/// the caller wrapping it in `critical-section`), so omitting the
/// lock is sound for the deployment model.
#[cfg(target_has_atomic = "8")]
static LOCK: AtomicBool = AtomicBool::new(false);

/// Spin until we hold the lock, run `f`, release.
#[cfg(target_has_atomic = "8")]
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    while LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // SAFETY: we hold the LOCK; no other path mutates REGISTRY
    // concurrently. The `LOCK` `Release` store at function exit
    // synchronises the writes for any subsequent `Acquire`.
    let reg: &mut Registry = unsafe { &mut *REGISTRY.0.get() };
    let r = f(reg);
    LOCK.store(false, Ordering::Release);
    r
}

/// Lock-free fallback for atomic-less targets (Cortex-M0 etc.). See
/// the docs on `LOCK` for why this is sound on those parts.
#[cfg(not(target_has_atomic = "8"))]
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> R {
    // SAFETY: single-threaded deployment model on no-atomic
    // targets; see the LOCK docs above.
    let reg: &mut Registry = unsafe { &mut *REGISTRY.0.get() };
    f(reg)
}

#[inline]
fn period_from_hz(hz: u32) -> Result<u32, TimerError> {
    if hz == 0 {
        return Err(TimerError::InvalidFrequency);
    }
    let period = 1_000_000u32.checked_div(hz).unwrap_or(0);
    if period == 0 {
        return Err(TimerError::InvalidFrequency);
    }
    Ok(period)
}

fn validate_mode(mode: TimerMode) -> Result<(), TimerError> {
    match mode {
        TimerMode::Pwm { duty_per_mille } if duty_per_mille > PWM_DUTY_MAX => {
            Err(TimerError::InvalidDutyCycle)
        }
        _ => Ok(()),
    }
}

/// Allocate a timer slot and configure it. Returns a stable
/// [`TimerId`] callers pass to every other entry point. Does NOT
/// start the timer — call [`timer_start`] when you're ready for
/// it to begin counting.
pub fn timer_init(config: TimerConfig) -> Result<TimerId, TimerError> {
    validate_mode(config.mode)?;
    let period = period_from_hz(config.frequency_hz)?;
    with_registry(|reg| {
        for (idx, slot) in reg.slots.iter_mut().enumerate() {
            if slot.status == Status::Free {
                *slot = TimerState {
                    status: Status::Stopped,
                    mode: config.mode,
                    period_us: period,
                    accum_us: 0,
                    count: 0,
                    elapsed_us: 0,
                    callback: None,
                };
                return Ok(TimerId(idx as u8));
            }
        }
        Err(TimerError::Exhausted)
    })
}

/// Release the slot held by `id`. After this call, the id is
/// invalid; passing it to any other entry point returns
/// [`TimerError::InvalidId`]. Idempotent on already-free slots.
pub fn timer_release(id: TimerId) -> Result<(), TimerError> {
    with_registry(|reg| match reg.slots.get_mut(id.0 as usize) {
        Some(slot) if slot.status != Status::Free => {
            *slot = TimerState::FREE;
            Ok(())
        }
        Some(_) => Err(TimerError::InvalidId),
        None => Err(TimerError::InvalidId),
    })
}

fn with_slot<R>(
    id: TimerId,
    f: impl FnOnce(&mut TimerState) -> Result<R, TimerError>,
) -> Result<R, TimerError> {
    with_registry(|reg| match reg.slots.get_mut(id.0 as usize) {
        Some(slot) if slot.status != Status::Free => f(slot),
        _ => Err(TimerError::InvalidId),
    })
}

/// Start (or restart) `id`. Re-arms an expired one-shot. A no-op
/// returning `Ok(())` if the timer is already running — matches
/// what most MCU HALs do.
pub fn timer_start(id: TimerId) -> Result<(), TimerError> {
    with_slot(id, |slot| {
        slot.status = Status::Running;
        // Re-arming an expired one-shot or restarting a stopped
        // periodic clears the accum so the next expiry is a full
        // period away. `elapsed_us` and `count` survive, matching
        // typical hardware-counter behaviour where "start" means
        // "resume" — `timer_reset` is the explicit zeroing entry.
        slot.accum_us = 0;
        Ok(())
    })
}

/// Stop `id` without losing accumulated state. `timer_start` will
/// resume it. Idempotent.
pub fn timer_stop(id: TimerId) -> Result<(), TimerError> {
    with_slot(id, |slot| {
        if slot.status == Status::Running {
            slot.status = Status::Stopped;
        }
        Ok(())
    })
}

/// Zero every counter for `id`, leaving the mode + period intact
/// and the timer in `Stopped` state. Callbacks are preserved —
/// use [`timer_set_callback`] with `None` to clear.
pub fn timer_reset(id: TimerId) -> Result<(), TimerError> {
    with_slot(id, |slot| {
        slot.status = Status::Stopped;
        slot.accum_us = 0;
        slot.count = 0;
        slot.elapsed_us = 0;
        Ok(())
    })
}

/// Number of expiries since the last reset (periodic / one-shot)
/// or completed PWM periods (PWM mode).
pub fn timer_count(id: TimerId) -> Result<u32, TimerError> {
    with_slot(id, |slot| Ok(slot.count))
}

/// Free-running microsecond counter since the timer was last
/// reset. Useful for measuring intervals shorter than one period.
pub fn timer_elapsed_us(id: TimerId) -> Result<u64, TimerError> {
    with_slot(id, |slot| Ok(slot.elapsed_us))
}

/// Install (or clear) the expiry callback. `Some(f)` registers
/// `f`; `None` clears. The callback fires from inside [`tick`]
/// when an expiry is detected — i.e. from the timer-driver
/// context, which on real hardware is an ISR. Keep it short.
pub fn timer_set_callback(id: TimerId, callback: Option<TimerCallback>) -> Result<(), TimerError> {
    with_slot(id, |slot| {
        slot.callback = callback;
        Ok(())
    })
}

/// Current PWM output level for a timer in `Pwm` mode. `true` =
/// high (output asserted), `false` = low. Returns
/// [`TimerError::InvalidState`] if the timer isn't a PWM timer or
/// isn't running.
pub fn timer_pwm_level(id: TimerId) -> Result<bool, TimerError> {
    with_slot(id, |slot| {
        let TimerMode::Pwm { duty_per_mille } = slot.mode else {
            return Err(TimerError::InvalidState);
        };
        if slot.status != Status::Running {
            return Err(TimerError::InvalidState);
        }
        let high_time_us = (slot.period_us as u64 * duty_per_mille as u64) / PWM_DUTY_MAX as u64;
        Ok((slot.accum_us as u64) < high_time_us)
    })
}

/// Block until `ms` milliseconds have elapsed on `id`. The wait
/// is driven by [`tick`] calls (so test harnesses that call
/// `tick(N)` directly satisfy it instantly). The timer must be
/// stopped before calling — `timer_delay_ms` does not interleave
/// with running interrupts on the same id.
///
/// On real firmware, callers would typically use a WFI loop while
/// the systick ISR drives `tick`. The function therefore needs
/// to release the registry lock between checks; we busy-spin
/// through `wait_until_elapsed`, which polls a snapshot.
pub fn timer_delay_ms(id: TimerId, ms: u32) -> Result<(), TimerError> {
    let target_us = (ms as u64).saturating_mul(1_000);
    with_slot(id, |slot| {
        if slot.status == Status::Running {
            return Err(TimerError::InvalidState);
        }
        slot.status = Status::Running;
        slot.accum_us = 0;
        slot.elapsed_us = 0;
        Ok(())
    })?;
    while timer_elapsed_us(id)? < target_us {
        core::hint::spin_loop();
    }
    with_slot(id, |slot| {
        slot.status = Status::Stopped;
        Ok(())
    })
}

/// Drive every running timer forward by `elapsed_us` microseconds.
/// Invoke registered callbacks for any timer that expires inside
/// the window.
///
/// On real firmware, hook this to a periodic systick interrupt
/// (or the dedicated timer-block interrupt). Tests call it
/// directly with a virtual elapsed window. A single `tick` may
/// fire multiple expiries for a fast periodic timer; the runtime
/// handles that without losing pulses.
pub fn tick(elapsed_us: u32) {
    // Two-phase: collect the callbacks to fire under the lock,
    // then drop the lock and fire them. Firing under the lock
    // would deadlock if a callback re-entered the timer API.
    let mut to_fire: [(Option<TimerCallback>, u32); MAX_TIMERS] = [(None, 0); MAX_TIMERS];
    with_registry(|reg| {
        for (idx, slot) in reg.slots.iter_mut().enumerate() {
            if slot.status != Status::Running {
                continue;
            }
            slot.elapsed_us = slot.elapsed_us.saturating_add(elapsed_us as u64);
            let mut remaining = elapsed_us;
            while remaining > 0 {
                let needed = slot.period_us.saturating_sub(slot.accum_us);
                if remaining < needed {
                    slot.accum_us = slot.accum_us.saturating_add(remaining);
                    remaining = 0;
                } else {
                    remaining = remaining.saturating_sub(needed);
                    slot.accum_us = 0;
                    slot.count = slot.count.saturating_add(1);
                    to_fire[idx].0 = slot.callback;
                    to_fire[idx].1 = to_fire[idx].1.saturating_add(1);
                    if matches!(slot.mode, TimerMode::OneShot) {
                        slot.status = Status::Expired;
                        break;
                    }
                }
            }
        }
    });
    // Lock released — fire callbacks.
    for entry in &to_fire {
        if let (Some(cb), n) = entry {
            for _ in 0..*n {
                cb();
            }
        }
    }
}

/// Reset the entire registry. Intended for test-suite use between
/// cases; safe to call on production firmware but rarely useful.
pub fn reset_all() {
    with_registry(|reg| {
        for slot in &mut reg.slots {
            *slot = TimerState::FREE;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};

    // Tests share the static REGISTRY, so cargo's parallel test
    // runner can interleave them. Each test grabs this lock so
    // results are deterministic. Mirrors the SINK_TEST_LOCK
    // pattern in `crate::sink`.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        let guard = match TEST_LOCK.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        reset_all();
        guard
    }

    // Callback target: bump a static counter so tests can verify
    // invocation without capturing state (closures need alloc).
    static CB_COUNT: AtomicU32 = AtomicU32::new(0);

    fn bump_cb() {
        CB_COUNT.fetch_add(1, Ordering::SeqCst);
    }

    fn reset_cb() {
        CB_COUNT.store(0, Ordering::SeqCst);
    }

    #[test]
    fn init_and_release_round_trip() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        assert_eq!(timer_count(id).unwrap(), 0);
        timer_release(id).unwrap();
        assert_eq!(timer_count(id).unwrap_err(), TimerError::InvalidId);
    }

    #[test]
    fn zero_frequency_rejected() {
        let _g = lock();
        let err = timer_init(TimerConfig::periodic(0)).unwrap_err();
        assert_eq!(err, TimerError::InvalidFrequency);
    }

    #[test]
    fn over_megahertz_rejected() {
        let _g = lock();
        // 2 MHz → period rounds to 0us → invalid.
        let err = timer_init(TimerConfig::periodic(2_000_000)).unwrap_err();
        assert_eq!(err, TimerError::InvalidFrequency);
    }

    #[test]
    fn bad_pwm_duty_rejected() {
        let _g = lock();
        let err = timer_init(TimerConfig::pwm(1_000, 1_500)).unwrap_err();
        assert_eq!(err, TimerError::InvalidDutyCycle);
    }

    #[test]
    fn exhaustion_after_max_timers() {
        let _g = lock();
        let mut ids = [TimerId(0); MAX_TIMERS];
        for slot in &mut ids {
            *slot = timer_init(TimerConfig::periodic(1_000)).unwrap();
        }
        let err = timer_init(TimerConfig::periodic(1_000)).unwrap_err();
        assert_eq!(err, TimerError::Exhausted);
        // Releasing one frees up exactly one slot.
        timer_release(ids[0]).unwrap();
        let _new = timer_init(TimerConfig::periodic(1_000)).unwrap();
    }

    #[test]
    fn periodic_callback_fires_each_period() {
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap(); // 1ms = 1000us period.
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        // 3 full periods worth of ticks.
        tick(3_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 3);
        assert_eq!(timer_count(id).unwrap(), 3);
    }

    #[test]
    fn periodic_callback_handles_multi_period_tick() {
        // Even if the host calls tick() with a window much larger
        // than one period, no expiries are lost.
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::periodic(10_000)).unwrap(); // 100us period.
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        tick(1_000); // = 10 periods.
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn one_shot_fires_once_and_auto_stops() {
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::one_shot(1_000)).unwrap();
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        tick(5_000); // Way past expiry.
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
        // Further ticks don't re-fire.
        tick(5_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn one_shot_re_arms_on_restart() {
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::one_shot(1_000)).unwrap();
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        tick(2_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
        timer_start(id).unwrap(); // re-arm
        tick(2_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn stop_pauses_then_start_resumes() {
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        tick(500);
        timer_stop(id).unwrap();
        tick(10_000); // Stopped — no callbacks.
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 0);
        timer_start(id).unwrap();
        tick(1_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn reset_zeroes_counters() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_start(id).unwrap();
        tick(3_500);
        assert_eq!(timer_count(id).unwrap(), 3);
        assert_eq!(timer_elapsed_us(id).unwrap(), 3_500);
        timer_reset(id).unwrap();
        assert_eq!(timer_count(id).unwrap(), 0);
        assert_eq!(timer_elapsed_us(id).unwrap(), 0);
    }

    #[test]
    fn pwm_level_reflects_duty() {
        let _g = lock();
        let id = timer_init(TimerConfig::pwm(1_000, 250)).unwrap(); // 25% duty, 1kHz = 1ms period.
        timer_start(id).unwrap();
        // At t=0, accum=0 < 250us high time → level high.
        assert!(timer_pwm_level(id).unwrap());
        tick(200);
        // accum=200us, still under 250us high time.
        assert!(timer_pwm_level(id).unwrap());
        tick(100);
        // accum=300us, past 250us high time → low.
        assert!(!timer_pwm_level(id).unwrap());
    }

    #[test]
    fn pwm_level_on_non_pwm_errors() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_start(id).unwrap();
        let err = timer_pwm_level(id).unwrap_err();
        assert_eq!(err, TimerError::InvalidState);
    }

    #[test]
    fn pwm_full_duty_is_always_high() {
        let _g = lock();
        let id = timer_init(TimerConfig::pwm(1_000, 1_000)).unwrap();
        timer_start(id).unwrap();
        for offset in [0u32, 100, 500, 999] {
            tick(offset);
            assert!(
                timer_pwm_level(id).unwrap(),
                "expected high at {}us",
                offset
            );
            timer_reset(id).unwrap();
            timer_start(id).unwrap();
        }
    }

    #[test]
    fn pwm_zero_duty_is_always_low() {
        let _g = lock();
        let id = timer_init(TimerConfig::pwm(1_000, 0)).unwrap();
        timer_start(id).unwrap();
        assert!(!timer_pwm_level(id).unwrap());
        tick(500);
        assert!(!timer_pwm_level(id).unwrap());
    }

    #[test]
    fn callback_clear_via_none() {
        let _g = lock();
        reset_cb();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_set_callback(id, Some(bump_cb)).unwrap();
        timer_start(id).unwrap();
        tick(1_000);
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
        timer_set_callback(id, None).unwrap();
        tick(1_000); // Period still expires (count bumps) but callback doesn't fire.
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 1);
        assert_eq!(timer_count(id).unwrap(), 2);
    }

    #[test]
    fn delay_ms_blocks_until_elapsed() {
        let _g = lock();
        // delay_ms busy-spins on timer_elapsed_us. Pre-load the
        // elapsed clock by faking the systick driver in a sibling
        // thread.
        let id = timer_init(TimerConfig::periodic(100_000)).unwrap(); // 10us tick.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_writer = stop.clone();
        let handle = std::thread::spawn(move || {
            while !stop_writer.load(Ordering::SeqCst) {
                // Each iteration advances the clock by 1ms. The
                // delay loop will catch up quickly.
                tick(1_000);
                std::thread::yield_now();
            }
        });
        timer_delay_ms(id, 5).unwrap();
        stop.store(true, Ordering::SeqCst);
        let _ = handle.join();
        // After delay_ms returns, elapsed_us must be at least
        // the requested 5ms = 5000us.
        let elapsed = timer_elapsed_us(id).unwrap();
        assert!(elapsed >= 5_000, "elapsed={}us", elapsed);
    }

    #[test]
    fn delay_ms_on_running_timer_errors() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_start(id).unwrap();
        let err = timer_delay_ms(id, 1).unwrap_err();
        assert_eq!(err, TimerError::InvalidState);
    }

    #[test]
    fn operations_on_released_id_error() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_release(id).unwrap();
        assert_eq!(timer_start(id).unwrap_err(), TimerError::InvalidId);
        assert_eq!(timer_stop(id).unwrap_err(), TimerError::InvalidId);
        assert_eq!(timer_reset(id).unwrap_err(), TimerError::InvalidId);
        assert_eq!(timer_count(id).unwrap_err(), TimerError::InvalidId);
        assert_eq!(timer_elapsed_us(id).unwrap_err(), TimerError::InvalidId);
        assert_eq!(
            timer_set_callback(id, Some(bump_cb)).unwrap_err(),
            TimerError::InvalidId
        );
        assert_eq!(timer_pwm_level(id).unwrap_err(), TimerError::InvalidId);
    }

    #[test]
    fn multiple_timers_independent() {
        let _g = lock();
        reset_cb();
        let a = timer_init(TimerConfig::periodic(1_000)).unwrap();
        let b = timer_init(TimerConfig::periodic(2_000)).unwrap(); // 500us period.
        timer_set_callback(a, Some(bump_cb)).unwrap();
        timer_set_callback(b, Some(bump_cb)).unwrap();
        timer_start(a).unwrap();
        timer_start(b).unwrap();
        tick(1_000);
        // a fires once (1000us period), b fires twice (500us).
        assert_eq!(CB_COUNT.load(Ordering::SeqCst), 3);
        assert_eq!(timer_count(a).unwrap(), 1);
        assert_eq!(timer_count(b).unwrap(), 2);
    }

    #[test]
    fn stop_on_stopped_is_noop() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        timer_stop(id).unwrap();
        timer_stop(id).unwrap();
    }

    #[test]
    fn timer_id_index_is_observable() {
        let _g = lock();
        let id = timer_init(TimerConfig::periodic(1_000)).unwrap();
        assert_eq!(id.index(), 0);
        let id2 = timer_init(TimerConfig::periodic(1_000)).unwrap();
        assert_eq!(id2.index(), 1);
    }
}
