use resilient_runtime::timer::{self, TimerConfig, TimerError, TimerId};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());
static CALLBACKS: AtomicU32 = AtomicU32::new(0);

fn callback() {
    CALLBACKS.fetch_add(1, Ordering::SeqCst);
}

fn spawn_wait(
    id: TimerId,
    ms: u32,
) -> (
    mpsc::Receiver<Result<(), TimerError>>,
    thread::JoinHandle<()>,
) {
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let _ = sender.send(timer::timer_delay_ms(id, ms));
    });
    (receiver, worker)
}

fn cleanup_timeout(
    id: TimerId,
    receiver: &mpsc::Receiver<Result<(), TimerError>>,
    worker: thread::JoinHandle<()>,
) -> ! {
    let _ = timer::timer_release(id);
    let _ = receiver.recv_timeout(Duration::from_secs(1));
    worker.join().unwrap();
    panic!("timer delay failed to complete or acknowledge cancellation");
}

fn drive_to_completion(id: TimerId, ms: u32) -> Result<(), TimerError> {
    let (receiver, worker) = spawn_wait(id, ms);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match receiver.try_recv() {
            Ok(result) => {
                worker.join().unwrap();
                return result;
            }
            Err(mpsc::TryRecvError::Disconnected) => panic!("waiter disconnected"),
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            cleanup_timeout(id, &receiver, worker);
        }
        timer::tick(1_000);
        thread::yield_now();
    }
}

fn wait_for_start(id: TimerId) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while timer::timer_elapsed_us(id).unwrap() == 0 {
        assert!(Instant::now() < deadline, "delay never started");
        timer::tick(1);
        thread::yield_now();
    }
}

#[test]
fn one_shot_delay_outlives_expiry_without_repeating_callbacks() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    CALLBACKS.store(0, Ordering::SeqCst);
    let id = timer::timer_init(TimerConfig::one_shot(1_000)).unwrap();
    timer::timer_set_callback(id, Some(callback)).unwrap();
    assert_eq!(drive_to_completion(id, 5), Ok(()));
    assert_eq!(timer::timer_count(id), Ok(1));
    assert_eq!(CALLBACKS.load(Ordering::SeqCst), 1);
    timer::timer_release(id).unwrap();
}

#[test]
fn periodic_delay_still_completes() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    let id = timer::timer_init(TimerConfig::periodic(1)).unwrap();
    assert_eq!(drive_to_completion(id, 5), Ok(()));
    timer::timer_release(id).unwrap();
}

fn cancellation(reset: bool) {
    let id = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
    let (receiver, worker) = spawn_wait(id, 1_000);
    wait_for_start(id);
    let competing_start = timer::timer_start(id);
    let competing_delay = timer::timer_delay_ms(id, 0);
    if reset {
        timer::timer_reset(id).unwrap();
    } else {
        timer::timer_stop(id).unwrap();
    }
    let result = match receiver.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result,
        Err(_) => cleanup_timeout(id, &receiver, worker),
    };
    worker.join().unwrap();
    assert_eq!(result, Err(TimerError::InvalidState));
    assert_eq!(competing_start, Err(TimerError::InvalidState));
    assert_eq!(competing_delay, Err(TimerError::InvalidState));
    timer::timer_start(id).unwrap();
    timer::timer_release(id).unwrap();
}

#[test]
fn stopped_delay_acknowledges_cancellation() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    cancellation(false);
}

#[test]
fn reset_delay_acknowledges_cancellation() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    cancellation(true);
}

#[test]
fn released_delay_rejects_its_invalid_handle() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    let id = timer::timer_init(TimerConfig::periodic(1_000)).unwrap();
    let (receiver, worker) = spawn_wait(id, 1_000);
    wait_for_start(id);
    timer::timer_release(id).unwrap();
    let result = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    worker.join().unwrap();
    assert_eq!(result, Err(TimerError::InvalidId));
}

#[test]
fn zero_delay_needs_no_driver_tick() {
    let _guard = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    timer::reset_all();
    let id = timer::timer_init(TimerConfig::one_shot(1_000)).unwrap();
    assert_eq!(timer::timer_delay_ms(id, 0), Ok(()));
    timer::timer_release(id).unwrap();
}
