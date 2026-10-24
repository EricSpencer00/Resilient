use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

const OUTPUT_LIMIT: usize = 64 * 1024;

struct SolverChild {
    child: Child,
    reaped: bool,
}

impl Drop for SolverChild {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn read_bounded(reader: impl Read) -> crate::RResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take((OUTPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read z3 output: {error}"))?;
    if bytes.len() > OUTPUT_LIMIT {
        return Err("z3 replay exceeded the 64 KiB per-stream output limit".into());
    }
    Ok(bytes)
}

fn timeout() -> crate::RResult<Duration> {
    let value = match std::env::var("RZ_SOLVER_TIMEOUT_MS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => return Ok(Duration::from_secs(30)),
        Err(_) => return Err("RZ_SOLVER_TIMEOUT_MS must be valid UTF-8".into()),
    };
    let millis = value
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=300_000).contains(value))
        .ok_or_else(|| "RZ_SOLVER_TIMEOUT_MS must be between 1 and 300000".to_string())?;
    Ok(Duration::from_millis(millis))
}

pub(crate) fn run(binary: &OsStr, certificate: &Path) -> crate::RResult<bool> {
    let budget = timeout()?;
    let start = Instant::now();
    let child = Command::new(binary)
        .arg("-smt2")
        .arg(certificate)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not spawn z3: {error}"))?;
    let mut process = SolverChild {
        child,
        reaped: false,
    };
    let stdout = process
        .child
        .stdout
        .take()
        .ok_or("missing z3 stdout pipe")?;
    let stderr = process
        .child
        .stderr
        .take()
        .ok_or("missing z3 stderr pipe")?;
    let (sender, receiver) = mpsc::channel();
    let stdout_sender = sender.clone();
    std::thread::Builder::new()
        .name("z3-stdout".into())
        .spawn(move || {
            let _ = stdout_sender.send((true, read_bounded(stdout)));
        })
        .map_err(|error| format!("could not start z3 output reader: {error}"))?;
    std::thread::Builder::new()
        .name("z3-stderr".into())
        .spawn(move || {
            let _ = sender.send((false, read_bounded(stderr)));
        })
        .map_err(|error| format!("could not start z3 output reader: {error}"))?;

    let (mut stdout, mut stderr, mut status) = (None, None, None);
    loop {
        let remaining = budget
            .checked_sub(start.elapsed())
            .ok_or("z3 replay exceeded its time limit")?;
        if status.is_none() {
            status = process
                .child
                .try_wait()
                .map_err(|error| format!("could not wait for z3: {error}"))?;
            process.reaped = status.is_some();
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        let interval = remaining.min(Duration::from_millis(5));
        match receiver.recv_timeout(interval) {
            Ok((is_stdout, result)) => {
                if is_stdout {
                    stdout = Some(result?);
                } else {
                    stderr = Some(result?);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                if stdout.is_none() || stderr.is_none() {
                    return Err("z3 output reader ended without reporting".into());
                }
                // Closed output pipes do not imply that the process has exited.
                std::thread::sleep(interval);
            }
        }
    }
    let status = status.ok_or("missing z3 exit status")?;
    let stdout = stdout.ok_or("missing z3 stdout")?;
    let stderr = stderr.ok_or("missing z3 stderr")?;
    // An UNSAT prefix cannot authenticate a failed or ambiguous solver run.
    if !status.success() || !stderr.is_empty() {
        return Err("z3 replay failed or emitted diagnostics".into());
    }
    let stdout =
        std::str::from_utf8(&stdout).map_err(|_| "z3 replay returned invalid UTF-8".to_string())?;
    Ok(stdout.trim() == "unsat")
}
