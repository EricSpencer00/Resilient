//! Repeated cross-thread delivery plus bounded overlap rejection.
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use resilient_runtime::live_telemetry::{self, LiveTelemetryBackend};
use resilient_runtime::sink::{self, Sink, SinkErr};

static TEST_LOCK: Mutex<()> = Mutex::new(());
const WORKERS: usize = 8;
const ROUNDS: usize = 16;
const LIMIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Hook {
    Sink,
    Telemetry,
}

impl Hook {
    fn invoke(self, owner: bool) -> Result<(), SinkErr> {
        let marker = if owner { "owner" } else { "contender" };
        match self {
            Self::Sink => sink::print(marker),
            Self::Telemetry => {
                live_telemetry::emit_live_retry(marker, 0, "handoff", 0);
                Ok(())
            }
        }
    }

    fn clear(self) {
        match self {
            Self::Sink => sink::clear_sink(),
            Self::Telemetry => live_telemetry::clear_live_telemetry(),
        }
    }
}

struct ControlledHook {
    hook: Hook,
    delivered: Arc<AtomicUsize>,
    faults: Arc<AtomicUsize>,
    entered: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}

impl ControlledHook {
    fn callback(&mut self, marker: &str) {
        if marker != "owner" {
            self.faults.fetch_add(1, Ordering::SeqCst);
            return;
        }
        self.delivered.fetch_add(1, Ordering::SeqCst);
        match self.hook {
            Hook::Sink => {
                if sink::print("nested") != Err(SinkErr::WriteFailed) {
                    self.faults.fetch_add(1, Ordering::SeqCst);
                }
            }
            Hook::Telemetry => live_telemetry::emit_live_retry("nested", 0, "reentry", 0),
        }
        if self.entered.send(()).is_err() || self.release.recv_timeout(LIMIT).is_err() {
            self.faults.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl Sink for ControlledHook {
    fn write_str(&mut self, marker: &str) -> Result<(), SinkErr> {
        self.callback(marker);
        Ok(())
    }
}

impl LiveTelemetryBackend for ControlledHook {
    fn on_live_retry(&mut self, block: &str, _: usize, _: &str, _: u64) {
        self.callback(block);
    }
}

fn exercise(hook: Hook) -> Vec<String> {
    let delivered = Arc::new(AtomicUsize::new(0));
    let faults = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let backend = Box::leak(Box::new(ControlledHook {
        hook,
        delivered: Arc::clone(&delivered),
        faults: Arc::clone(&faults),
        entered: entered_tx,
        release: release_rx,
    }));
    hook.clear();
    match hook {
        Hook::Sink => sink::set_sink(backend),
        Hook::Telemetry => live_telemetry::set_live_telemetry(backend),
    }

    let (completed_tx, completed_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let mut commands = Vec::new();
    let mut handles = Vec::new();
    for id in 0..WORKERS {
        let (command_tx, command_rx) = mpsc::channel::<Option<bool>>();
        commands.push(command_tx);
        let completed = completed_tx.clone();
        let done = done_tx.clone();
        handles.push(thread::spawn(move || {
            while let Ok(Some(owner)) = command_rx.recv_timeout(LIMIT + LIMIT) {
                let result = hook.invoke(owner);
                if completed.send((id, owner, result)).is_err() {
                    break;
                }
            }
            let _ = done.send(id);
        }));
    }
    drop(completed_tx);
    drop(done_tx);

    let mut errors = Vec::new();
    let mut positives = [0; WORKERS];
    'rounds: for _ in 0..ROUNDS {
        for owner in 0..WORKERS {
            let expected_count = positives.iter().sum::<usize>() + 1;
            let held = (|| -> Result<(), String> {
                commands[owner]
                    .send(Some(true))
                    .map_err(|e| e.to_string())?;
                entered_rx
                    .recv_timeout(LIMIT)
                    .map_err(|e| format!("owner {owner} never entered: {e}"))?;
                for (id, command) in commands.iter().enumerate() {
                    if id != owner {
                        command.send(Some(false)).map_err(|e| e.to_string())?;
                    }
                }
                let deadline = Instant::now() + LIMIT;
                let mut seen = BTreeSet::new();
                for _ in 1..WORKERS {
                    let (id, positive, result) = completed_rx
                        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .map_err(|e| format!("contender did not finish while owner held: {e}"))?;
                    if id == owner || positive || !seen.insert(id) {
                        return Err("unexpected or duplicate contender completion".into());
                    }
                    if matches!(hook, Hook::Sink) && result != Err(SinkErr::WriteFailed) {
                        return Err(format!("wrong overlapping sink result: {result:?}"));
                    }
                }
                if delivered.load(Ordering::SeqCst) != expected_count
                    || faults.load(Ordering::SeqCst) != 0
                {
                    return Err("unexpected callback, reentry, or release timeout".into());
                }
                Ok(())
            })();
            // Release before checking outcomes so a failed assertion cannot strand the owner.
            let _ = release_tx.send(());
            if let Err(error) = held {
                errors.push(error);
                break 'rounds;
            }
            match completed_rx.recv_timeout(LIMIT) {
                Ok((id, true, Ok(()))) if id == owner => positives[owner] += 1,
                result => {
                    errors.push(format!("positive owner delivery failed: {result:?}"));
                    break 'rounds;
                }
            }
        }
    }

    let _ = release_tx.send(());
    for command in &commands {
        let _ = command.send(None);
    }
    let deadline = Instant::now() + LIMIT + LIMIT;
    let mut done = BTreeSet::new();
    while done.len() < WORKERS {
        match done_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(id) => {
                done.insert(id);
            }
            Err(error) => {
                errors.push(format!("worker cleanup incomplete: {error}"));
                break;
            }
        }
    }
    // Worker acknowledgements follow their last hook call. Avoid unbounded joins
    // even on a failed protocol; missing acknowledgements already fail the test.
    drop(handles);
    hook.clear();
    if positives != [ROUNDS; WORKERS] || delivered.load(Ordering::SeqCst) != WORKERS * ROUNDS {
        errors.push(format!(
            "expected 16 deliveries per persistent worker: {positives:?}"
        ));
    }
    if faults.load(Ordering::SeqCst) != 0 {
        errors.push("callback protocol or reentry failure".into());
    }
    errors
}

#[test]
fn sink_serializes_parallel_writes_and_fails_closed_on_reentry() {
    let _lock = TEST_LOCK.lock().unwrap();
    let errors = exercise(Hook::Sink);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(sink::print("after clear"), Err(SinkErr::NoSink));
}

#[test]
fn telemetry_serializes_parallel_events_and_fails_closed_on_reentry() {
    let _lock = TEST_LOCK.lock().unwrap();
    let errors = exercise(Hook::Telemetry);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!live_telemetry::has_live_telemetry_backend());
}
