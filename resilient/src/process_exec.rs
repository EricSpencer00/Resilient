//! RES-2558: Process spawning and command execution (std-only).
//!
//! * `exec(cmd, args) -> Result<ProcessResult, string>`
//!   — Run `cmd` with `args`; capture stdout/stderr/exit_code.
//! * `exec_shell(cmd) -> Result<ProcessResult, string>`
//!   — Run `cmd` via the system shell (`sh -c` on Unix, `cmd /c` on Windows).
//!
//! `ProcessResult` is returned as a `Value::Struct` with fields:
//! - `stdout: string` — captured standard output
//! - `stderr: string` — captured standard error
//! - `exit_code: int` — process exit status (0 = success)
//!
//! Non-zero exit codes are NOT errors — check `exit_code` yourself.
//! A `Result::Err` is only returned when the process cannot be spawned
//! (e.g., command not found, permission denied).
//!
//! RES-4126: on `wasm32` (the web playground) there is no host
//! process to spawn — `std::process::Command` is unimplemented for
//! that target. Both builtins validate their arguments and then
//! short-circuit with a graceful "unsupported on this target" `Err`
//! instead of failing to build.
//!
//! RES-4820: captured stdout and stderr are each capped at
//! `MAX_PROCESS_OUTPUT_BYTES`. A child that exceeds the cap is killed and
//! the builtin returns `Err` instead of a partial result, so an untrusted
//! process cannot force an unbounded host allocation.

use crate::Value;

type RResult<T> = Result<T, String>;

#[cfg(not(target_arch = "wasm32"))]
use std::io::{self, Read};
#[cfg(not(target_arch = "wasm32"))]
use std::process::{Child, Command, Output, Stdio};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;

/// RES-4820: per-stream cap on captured child output (16 MiB). Matches the
/// socket receive budget in `tcp_udp.rs`.
#[cfg(not(target_arch = "wasm32"))]
const MAX_PROCESS_OUTPUT_BYTES: u64 = 16 * 1024 * 1024;

/// How often the collector checks the child's exit status while readers run.
#[cfg(not(target_arch = "wasm32"))]
const COLLECT_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
enum CaptureError {
    Spawn(io::Error),
    OutputTooLarge {
        stream: &'static str,
        limit: u64,
    },
    Io {
        stream: &'static str,
        source: io::Error,
    },
    Wait(io::Error),
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "failed to spawn: {e}"),
            Self::OutputTooLarge { stream, limit } => write!(
                f,
                "{stream} exceeded the {limit}-byte output limit; process was killed"
            ),
            Self::Io { stream, source } => write!(f, "failed to read {stream}: {source}"),
            Self::Wait(e) => write!(f, "failed to wait for process: {e}"),
        }
    }
}

/// Reads one pipe to EOF, failing as soon as it yields more than `limit`
/// bytes. `take(limit + 1)` bounds the allocation; one extra byte is enough
/// to distinguish "exactly at budget" from "over budget".
#[cfg(not(target_arch = "wasm32"))]
fn read_bounded<R: Read>(
    pipe: R,
    stream: &'static str,
    limit: u64,
) -> Result<Vec<u8>, CaptureError> {
    let mut buf = Vec::new();
    pipe.take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|source| CaptureError::Io { stream, source })?;
    if buf.len() as u64 > limit {
        return Err(CaptureError::OutputTooLarge { stream, limit });
    }
    Ok(buf)
}

/// Each pipe gets its own thread so neither can stall the other: a child
/// blocked writing stderr cannot prevent us from draining stdout, and vice
/// versa. A failed reader drops its pipe, which closes the read end.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    stream: &'static str,
    limit: u64,
    tx: Sender<(&'static str, Result<Vec<u8>, CaptureError>)>,
) {
    thread::spawn(move || {
        let result = match pipe {
            Some(pipe) => read_bounded(pipe, stream, limit),
            None => Ok(Vec::new()),
        };
        // The collector stops listening after the first failure; a send
        // error then just means nobody needs this result.
        let _ = tx.send((stream, result));
    });
}

/// Waits for the child and both bounded readers. Returns on the first
/// reader failure without waiting for the other stream to close, so the
/// caller can kill a child that is still writing.
#[cfg(not(target_arch = "wasm32"))]
fn collect_output(
    child: &mut Child,
    rx: &Receiver<(&'static str, Result<Vec<u8>, CaptureError>)>,
) -> Result<Output, CaptureError> {
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    while status.is_none() || stdout.is_none() || stderr.is_none() {
        if status.is_none() {
            status = child.try_wait().map_err(CaptureError::Wait)?;
        }
        match rx.recv_timeout(COLLECT_POLL_INTERVAL) {
            Ok(("stdout", result)) => stdout = Some(result?),
            Ok((_, result)) => stderr = Some(result?),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(CaptureError::Wait(io::Error::other(
                    "output reader thread ended without reporting",
                )));
            }
        }
    }
    match (status, stdout, stderr) {
        (Some(status), Some(stdout), Some(stderr)) => Ok(Output {
            status,
            stdout,
            stderr,
        }),
        _ => Err(CaptureError::Wait(io::Error::other(
            "output collection ended before the process and both streams finished",
        ))),
    }
}

/// Runs `cmd` with stdin closed and stdout/stderr captured, each bounded by
/// `limit`. Same contract as `Command::output()` for in-budget output.
#[cfg(not(target_arch = "wasm32"))]
fn capture_bounded(mut cmd: Command, limit: u64) -> Result<Output, CaptureError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(CaptureError::Spawn)?;
    let (tx, rx) = mpsc::channel();
    spawn_reader(child.stdout.take(), "stdout", limit, tx.clone());
    spawn_reader(child.stderr.take(), "stderr", limit, tx);
    let result = collect_output(&mut child, &rx);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(not(target_arch = "wasm32"))]
fn process_result_from_output(out: Output) -> Value {
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let exit_code = out.status.code().unwrap_or(-1) as i64;
    make_process_result(stdout, stderr, exit_code)
}

fn make_process_result(stdout: String, stderr: String, exit_code: i64) -> Value {
    Value::Struct {
        name: "ProcessResult".to_string(),
        fields: vec![
            ("stdout".to_string(), Value::String(stdout)),
            ("stderr".to_string(), Value::String(stderr)),
            ("exit_code".to_string(), Value::Int(exit_code)),
        ],
    }
}

fn ok(v: Value) -> Value {
    Value::Result {
        ok: true,
        payload: Box::new(v),
    }
}

fn err(msg: String) -> Value {
    Value::Result {
        ok: false,
        payload: Box::new(Value::String(msg)),
    }
}

#[cfg(target_arch = "wasm32")]
fn unsupported(builtin: &str) -> Value {
    err(format!(
        "{}: unsupported on this target (no host process spawning in the wasm playground)",
        builtin
    ))
}

/// `exec(cmd, args) -> Result<ProcessResult, string>`
///
/// Runs `cmd` with the given argument list. Returns `Ok(ProcessResult)` on
/// successful spawn regardless of exit code; returns `Err(reason)` only when
/// the process cannot be started (command not found, permission denied, etc.).
///
/// ```text
/// let r = exec("echo", ["hello"]);
/// println(unwrap(r).stdout);  // "hello\n"
/// ```
pub(crate) fn builtin_exec(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(cmd), Value::Array(arg_vals)] => {
            let mut str_args: Vec<String> = Vec::with_capacity(arg_vals.len());
            for (i, v) in arg_vals.iter().enumerate() {
                match v {
                    Value::String(s) => str_args.push(s.clone()),
                    Value::Int(n) => str_args.push(n.to_string()),
                    other => {
                        return Err(format!(
                            "exec: argument {} must be a string or int, got {}",
                            i, other
                        ));
                    }
                }
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (cmd, str_args);
                Ok(unsupported("exec"))
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let mut command = Command::new(cmd);
                command.args(&str_args);
                match capture_bounded(command, MAX_PROCESS_OUTPUT_BYTES) {
                    Ok(out) => Ok(ok(process_result_from_output(out))),
                    Err(CaptureError::Spawn(e)) => {
                        Ok(err(format!("exec: failed to spawn '{}': {}", cmd, e)))
                    }
                    Err(e) => Ok(err(format!("exec: {}", e))),
                }
            }
        }
        [Value::String(_), other] => Err(format!(
            "exec: second argument must be an array of strings, got {}",
            other
        )),
        [other, _] => Err(format!(
            "exec: first argument must be a string (command), got {}",
            other
        )),
        _ => Err(format!(
            "exec: expected 2 arguments (cmd, args), got {}",
            args.len()
        )),
    }
}

/// `exec_shell(cmd) -> Result<ProcessResult, string>`
///
/// Runs `cmd` through the system shell (`sh -c` on Unix, `cmd /c` on Windows).
/// Useful for pipelines and shell expansions.
///
/// ```text
/// let r = exec_shell("echo hello | wc -c");
/// println(unwrap(r).stdout);
/// ```
pub(crate) fn builtin_exec_shell(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(cmd)] => {
            #[cfg(target_arch = "wasm32")]
            {
                let _ = cmd;
                return Ok(unsupported("exec_shell"));
            }
            #[cfg(unix)]
            let result = capture_bounded(
                {
                    let mut c = Command::new("sh");
                    c.args(["-c", cmd.as_str()]);
                    c
                },
                MAX_PROCESS_OUTPUT_BYTES,
            );
            #[cfg(windows)]
            let result = capture_bounded(
                {
                    let mut c = Command::new("cmd");
                    c.args(["/c", cmd.as_str()]);
                    c
                },
                MAX_PROCESS_OUTPUT_BYTES,
            );
            #[cfg(not(any(unix, windows, target_arch = "wasm32")))]
            let result: Result<Output, CaptureError> = Err(CaptureError::Spawn(io::Error::new(
                io::ErrorKind::Unsupported,
                "exec_shell not supported on this platform",
            )));

            #[cfg(not(target_arch = "wasm32"))]
            match result {
                Ok(out) => Ok(ok(process_result_from_output(out))),
                Err(CaptureError::Spawn(e)) => {
                    Ok(err(format!("exec_shell: failed to spawn shell: {}", e)))
                }
                Err(e) => Ok(err(format!("exec_shell: {}", e))),
            }
        }
        [other] => Err(format!(
            "exec_shell: expected a string command, got {}",
            other
        )),
        _ => Err(format!(
            "exec_shell: expected 1 argument, got {}",
            args.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> crate::RunResult {
        run_program(src)
    }

    #[test]
    fn exec_echo_captures_stdout() {
        let r = run(r#"let r = exec("echo", ["hello"]);
let res = unwrap(r);
println(is_ok(r));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn exec_exit_code_zero_on_success() {
        let r = run(r#"let r = exec("true", []);
let res = unwrap(r);
println(res.exit_code);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains('0'), "stdout: {}", r.stdout);
    }

    #[test]
    fn exec_exit_code_nonzero_on_failure() {
        let r = run(r#"let r = exec("false", []);
let res = unwrap(r);
println(is_ok(r));
println(res.exit_code);"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        let lines: Vec<&str> = r.stdout.trim().lines().collect();
        // is_ok(r) should be true (spawn succeeded); exit_code is non-zero
        assert_eq!(lines[0], "true");
        // exit_code != 0
        assert_ne!(lines[1], "0", "expected non-zero exit code");
    }

    #[test]
    fn exec_bad_command_returns_err() {
        let r = run(
            r#"let r = exec("this_command_definitely_does_not_exist_on_any_system", []);
println(is_err(r));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[cfg(unix)]
    #[test]
    fn exec_shell_runs_pipeline() {
        let r = run(r#"let r = exec_shell("echo hello");
let res = unwrap(r);
println(trim(res.stdout));"#);
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("hello"), "stdout: {}", r.stdout);
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use super::*;

    #[test]
    fn exec_errs_on_wasm32() {
        let r = builtin_exec(&[Value::String("echo".to_string()), Value::Array(vec![])]).unwrap();
        assert!(matches!(r, Value::Result { ok: false, .. }));
    }

    #[test]
    fn exec_shell_errs_on_wasm32() {
        let r = builtin_exec_shell(&[Value::String("echo hello".to_string())]).unwrap();
        assert!(matches!(r, Value::Result { ok: false, .. }));
    }
}

#[cfg(all(test, unix))]
mod bounded_capture_tests {
    use super::*;
    use crate::run_program;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[test]
    fn capture_accepts_stdout_exactly_at_limit() {
        let out = capture_bounded(sh("printf abcdefgh"), 8).expect("at-limit output is valid");
        assert_eq!(out.stdout, b"abcdefgh");
        assert!(out.stderr.is_empty());
        assert!(out.status.success());
    }

    #[test]
    fn capture_accepts_stderr_exactly_at_limit() {
        let out = capture_bounded(sh("printf abcdefgh >&2"), 8).expect("at-limit output is valid");
        assert!(out.stdout.is_empty());
        assert_eq!(out.stderr, b"abcdefgh");
    }

    #[test]
    fn capture_rejects_stdout_one_byte_over_limit() {
        match capture_bounded(sh("printf abcdefghi"), 8) {
            Err(CaptureError::OutputTooLarge { stream, limit }) => {
                assert_eq!(stream, "stdout");
                assert_eq!(limit, 8);
            }
            other => panic!("expected stdout OutputTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn capture_rejects_stderr_one_byte_over_limit() {
        match capture_bounded(sh("printf abcdefghi >&2"), 8) {
            Err(CaptureError::OutputTooLarge { stream, .. }) => assert_eq!(stream, "stderr"),
            other => panic!("expected stderr OutputTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn capture_kills_runaway_child_once_limit_is_hit() {
        let mut cmd = Command::new("yes");
        cmd.stdin(Stdio::null());
        match capture_bounded(cmd, 1024) {
            Err(CaptureError::OutputTooLarge { stream, limit }) => {
                assert_eq!(stream, "stdout");
                assert_eq!(limit, 1024);
            }
            other => panic!("expected OutputTooLarge from `yes`, got {other:?}"),
        }
    }

    #[test]
    fn capture_drains_both_pipes_without_deadlock() {
        // Stderr is written first and is larger than a typical pipe buffer,
        // so a single-threaded reader that drained stdout first would hang.
        let script = "head -c 200000 /dev/zero >&2; head -c 200000 /dev/zero";
        let out = capture_bounded(sh(script), 1024 * 1024).expect("both streams fit the budget");
        assert_eq!(out.stdout.len(), 200_000);
        assert_eq!(out.stderr.len(), 200_000);
        assert!(out.status.success());
    }

    #[test]
    fn capture_preserves_exit_code_and_output() {
        let out = capture_bounded(sh("printf hi; exit 3"), 16).expect("in-budget output");
        assert_eq!(out.stdout, b"hi");
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn capture_empty_output_is_ok() {
        let out = capture_bounded(sh("true"), 16).expect("empty output is valid");
        assert!(out.stdout.is_empty());
        assert!(out.stderr.is_empty());
        assert!(out.status.success());
    }

    #[test]
    fn capture_spawn_error_is_reported_as_spawn() {
        let cmd = Command::new("this_command_definitely_does_not_exist_res_4820");
        assert!(matches!(
            capture_bounded(cmd, 16),
            Err(CaptureError::Spawn(_))
        ));
    }

    #[test]
    fn exec_shell_oversized_output_returns_err() {
        // 16 MiB + 1 on stdout: one byte past the builtin's budget.
        let r = run_program(
            r#"let r = exec_shell("head -c 16777217 /dev/zero");
println(is_err(r));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }

    #[test]
    fn exec_shell_output_at_budget_is_ok() {
        let r = run_program(
            r#"let r = exec_shell("head -c 16777216 /dev/zero");
println(is_ok(r));"#,
        );
        assert!(r.ok, "errors: {:?}", r.errors);
        assert!(r.stdout.contains("true"), "stdout: {}", r.stdout);
    }
}
