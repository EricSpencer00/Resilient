use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use resilient_runtime::live_telemetry::{self, LiveTelemetryBackend};
use resilient_runtime::sink::{self, Sink, SinkErr};

struct CountingSink {
    writes: Arc<AtomicUsize>,
}

impl Sink for CountingSink {
    fn write_str(&mut self, _s: &str) -> Result<(), SinkErr> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

struct ReentrantSink;

impl Sink for ReentrantSink {
    fn write_str(&mut self, _s: &str) -> Result<(), SinkErr> {
        assert_eq!(sink::print("nested"), Err(SinkErr::WriteFailed));
        Ok(())
    }
}

struct CountingBackend {
    events: Arc<AtomicUsize>,
}

impl LiveTelemetryBackend for CountingBackend {
    fn on_live_retry(&mut self, _block: &str, _retry: usize, _reason: &str, _ts_ns: u64) {
        self.events.fetch_add(1, Ordering::Relaxed);
    }
}

struct ReentrantBackend;

impl LiveTelemetryBackend for ReentrantBackend {
    fn on_live_retry(&mut self, _block: &str, _retry: usize, _reason: &str, _ts_ns: u64) {
        live_telemetry::emit_live_retry("nested", 0, "nested", 0);
    }
}

#[test]
fn sink_serializes_parallel_writes_and_fails_closed_on_reentry() {
    let writes = Arc::new(AtomicUsize::new(0));
    sink::set_sink(Box::leak(Box::new(CountingSink {
        writes: Arc::clone(&writes),
    })));

    let workers: Vec<_> = (0..8)
        .map(|_| {
            thread::spawn(|| {
                for _ in 0..16 {
                    sink::print("x").unwrap();
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(writes.load(Ordering::Relaxed), 128);

    sink::set_sink(Box::leak(Box::new(ReentrantSink)));
    sink::print("outer").unwrap();
    sink::clear_sink();
}

#[test]
fn telemetry_serializes_parallel_events_and_fails_closed_on_reentry() {
    let events = Arc::new(AtomicUsize::new(0));
    live_telemetry::set_live_telemetry(Box::leak(Box::new(CountingBackend {
        events: Arc::clone(&events),
    })));

    let workers: Vec<_> = (0..8)
        .map(|_| {
            thread::spawn(|| {
                for _ in 0..16 {
                    live_telemetry::emit_live_retry("block", 1, "reason", 1);
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(events.load(Ordering::Relaxed), 128);

    live_telemetry::set_live_telemetry(Box::leak(Box::new(ReentrantBackend)));
    live_telemetry::emit_live_retry("outer", 1, "reason", 1);
    live_telemetry::clear_live_telemetry();
}
