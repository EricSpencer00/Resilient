use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use resilient_runtime::live_telemetry::{self, LiveTelemetryBackend};
use resilient_runtime::sink::{self, Sink, SinkErr};

const DEADLINE: Duration = Duration::from_secs(5);

struct HeldCallback {
    calls: Arc<AtomicUsize>,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl HeldCallback {
    fn enter(&mut self) {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv_timeout(DEADLINE).unwrap();
        }
    }
}

impl Sink for HeldCallback {
    fn write_str(&mut self, _text: &str) -> Result<(), SinkErr> {
        self.enter();
        Ok(())
    }
}

impl LiveTelemetryBackend for HeldCallback {
    fn on_live_retry(&mut self, _block: &str, _retry: usize, _reason: &str, _ts_ns: u64) {
        self.enter();
    }
}

type CallbackHarness = (
    HeldCallback,
    Arc<AtomicUsize>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
);

fn held_callback() -> CallbackHarness {
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        HeldCallback {
            calls: Arc::clone(&calls),
            entered: entered_tx,
            release: Mutex::new(release_rx),
        },
        calls,
        entered_rx,
        release_tx,
    )
}

#[test]
fn sink_rejects_overlapping_write_and_preserves_installed_sink() {
    let (callback, calls, entered, release) = held_callback();
    sink::set_sink(Box::leak(Box::new(callback)));
    let held = thread::spawn(|| sink::print("held"));
    entered.recv_timeout(DEADLINE).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let contender = thread::spawn(move || {
        let result = sink::print("contended");
        sink::clear_sink();
        done_tx.send(result).unwrap();
    });
    let result = done_rx.recv_timeout(Duration::from_secs(1));
    // Release before assertions so a blocking-lock regression terminates cleanly.
    release.send(()).unwrap();
    held.join().unwrap().unwrap();
    contender.join().unwrap();
    assert_eq!(result.unwrap(), Err(SinkErr::WriteFailed));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    sink::print("after contention").unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    sink::clear_sink();
    assert_eq!(sink::print("cleared"), Err(SinkErr::NoSink));
}

#[test]
fn telemetry_drops_overlapping_event_and_preserves_installed_backend() {
    let (callback, calls, entered, release) = held_callback();
    live_telemetry::set_live_telemetry(Box::leak(Box::new(callback)));
    let held = thread::spawn(|| live_telemetry::emit_live_retry("held", 1, "reason", 0));
    entered.recv_timeout(DEADLINE).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let contender = thread::spawn(move || {
        live_telemetry::emit_live_retry("contended", 1, "reason", 0);
        live_telemetry::clear_live_telemetry();
        done_tx.send(()).unwrap();
    });
    let result = done_rx.recv_timeout(Duration::from_secs(1));
    release.send(()).unwrap();
    held.join().unwrap();
    contender.join().unwrap();
    result.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(live_telemetry::has_live_telemetry_backend());
    live_telemetry::emit_live_retry("after contention", 1, "reason", 0);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    live_telemetry::clear_live_telemetry();
    assert!(!live_telemetry::has_live_telemetry_backend());
}
