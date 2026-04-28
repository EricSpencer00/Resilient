// RES-228: `rz run --watch <file>` — re-run the program on every save.
//
// All watch-mode logic lives here per the feature-isolation pattern in
// CLAUDE.md.  The only changes to main.rs are:
//   1. `mod watch_mode;`
//   2. a dispatch call just before the normal `execute_file` path.
//
// Design notes:
// * Uses `notify` + `notify-debouncer-mini` for cross-platform file
//   watching with a 200 ms debounce — fast enough for interactive use,
//   long enough to coalesce multi-write saves.
// * The watcher thread delivers `DebounceEventResult` via a plain
//   `std::sync::mpsc` channel so no extra runtime dependency is needed.
// * CI / pipe detection: when the environment variable `CI` is set
//   (or `NO_COLOR` is set, both common in non-interactive pipelines)
//   we skip the watcher entirely and fall through to a single
//   `execute_file` call.  This keeps `--watch` safe to use in scripts
//   that happen to pass the flag through.

use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use notify_debouncer_mini::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

/// Entry point called from `main()` when `--watch` is passed.
///
/// Runs `file_path` immediately, then re-runs it on every save until
/// the user presses Ctrl-C.  `execute_once` is a closure that
/// encapsulates all the flags from the outer CLI parse — it mirrors the
/// `execute_file(…)` call that would have happened without `--watch`.
pub fn run_watch(file_path: &Path, execute_once: impl Fn() -> bool) {
    // Silently fall through to a single run when we detect a
    // non-interactive environment so CI pipelines that happen to pass
    // `--watch` are not surprised by a blocking loop.
    if is_non_interactive() {
        execute_once();
        return;
    }

    // First run before entering the watch loop.
    execute_once();

    let (tx, rx) = mpsc::channel::<DebounceEventResult>();

    let tx_clone = tx;
    let mut debouncer = match new_debouncer(
        Duration::from_millis(200),
        move |res: DebounceEventResult| {
            // Forward every result to the main thread; ignore send
            // errors that happen if the receiver has already been
            // dropped (i.e. after Ctrl-C).
            let _ = tx_clone.send(res);
        },
    ) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("watch: could not create file watcher: {}", e);
            return;
        }
    };

    // Watch the target file itself (non-recursive — it's a single file).
    if let Err(e) = debouncer
        .watcher()
        .watch(file_path, RecursiveMode::NonRecursive)
    {
        eprintln!("watch: could not watch {:?}: {}", file_path, e);
        return;
    }

    // Also watch the containing directory so that editors which write a
    // temp file then rename it (e.g. vim, emacs) still trigger a
    // notification for this path.
    if let Some(dir) = file_path.parent()
        && dir != Path::new("")
    {
        // Directory watch failure is non-fatal; the file watch above
        // still fires for direct writes.
        let _ = debouncer.watcher().watch(dir, RecursiveMode::NonRecursive);
    }

    eprintln!(
        "Watching {:?} for changes — press Ctrl-C to stop",
        file_path
    );

    for result in rx {
        match result {
            Ok(events) => {
                // Only re-run when at least one event touches our file.
                let relevant = events.iter().any(|ev| {
                    ev.path
                        .canonicalize()
                        .ok()
                        .or_else(|| Some(ev.path.clone()))
                        == file_path
                            .canonicalize()
                            .ok()
                            .or_else(|| Some(file_path.to_path_buf()))
                });
                if relevant {
                    eprintln!("--- [re-run: {}] ---", timestamp_now());
                    execute_once();
                }
            }
            Err(e) => {
                eprintln!("watch error: {:?}", e);
            }
        }
    }
}

/// Returns `true` when running in a known non-interactive context so
/// that `--watch` gracefully degrades to a single run instead of
/// blocking forever.
fn is_non_interactive() -> bool {
    // `CI` is set by GitHub Actions, GitLab CI, Travis, CircleCI, etc.
    std::env::var("CI").is_ok()
}

/// Returns a human-readable HH:MM:SS timestamp using only `std::time`.
fn timestamp_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    format!("{:02}:{:02}:{:02}", h, m, s)
}
