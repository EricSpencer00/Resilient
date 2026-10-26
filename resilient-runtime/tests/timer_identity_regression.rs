use resilient_runtime::timer::{self, TimerConfig, TimerError};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

static SERIAL: Mutex<()> = Mutex::new(());
static CALLBACKS: AtomicU32 = AtomicU32::new(0);

fn callback() {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn stale_handle_cannot_read_or_control_a_replacement_timer() {
    let _guard = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    timer::reset_all();
    let old = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
    timer::timer_release(old).unwrap();
    let fresh = timer::timer_init(TimerConfig::pwm(1_000, 500)).unwrap();
    assert_eq!(old.index(), fresh.index());
    assert_ne!(old, fresh);
    timer::timer_set_callback(fresh, Some(callback)).unwrap();
    CALLBACKS.store(0, Ordering::SeqCst);
    timer::timer_start(fresh).unwrap();
    timer::tick(1_000);
    assert_eq!(timer::timer_start(old), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_stop(old), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_reset(old), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_count(old), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_elapsed_us(old), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_pwm_level(old), Err(TimerError::InvalidId));
    assert_eq!(
        timer::timer_set_callback(old, None),
        Err(TimerError::InvalidId)
    );
    assert_eq!(timer::timer_delay_ms(old, 0), Err(TimerError::InvalidId));
    assert_eq!(timer::timer_release(old), Err(TimerError::InvalidId));
    timer::tick(1_000);
    assert_eq!(timer::timer_count(fresh), Ok(2));
    assert_eq!(timer::timer_elapsed_us(fresh), Ok(2_000));
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 2);
    timer::timer_release(fresh).unwrap();
}

#[test]
fn registry_resets_do_not_resurrect_prior_handles() {
    let _guard = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    timer::reset_all();
    let old = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
    for _ in 0..64 {
        timer::reset_all();
        let fresh = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
        assert_eq!(old.index(), fresh.index());
        assert_ne!(old, fresh);
        assert_eq!(timer::timer_start(old), Err(TimerError::InvalidId));
        assert_eq!(timer::timer_count(fresh), Ok(0));
    }
    timer::reset_all();
}

#[test]
fn timer_reset_preserves_the_live_handle() {
    let _guard = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    timer::reset_all();
    let id = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
    timer::timer_start(id).unwrap();
    timer::tick(2_000);
    timer::timer_reset(id).unwrap();
    assert_eq!(timer::timer_count(id), Ok(0));
    timer::timer_start(id).unwrap();
    timer::tick(1_000);
    assert_eq!(timer::timer_count(id), Ok(1));
    timer::timer_release(id).unwrap();
}
