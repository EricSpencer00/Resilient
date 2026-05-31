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

use crate::Value;

type RResult<T> = Result<T, String>;

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
            match std::process::Command::new(cmd).args(&str_args).output() {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                    let exit_code = out.status.code().unwrap_or(-1) as i64;
                    Ok(ok(make_process_result(stdout, stderr, exit_code)))
                }
                Err(e) => Ok(err(format!("exec: failed to spawn '{}': {}", cmd, e))),
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
            #[cfg(unix)]
            let result = std::process::Command::new("sh")
                .args(["-c", cmd.as_str()])
                .output();
            #[cfg(windows)]
            let result = std::process::Command::new("cmd")
                .args(["/c", cmd.as_str()])
                .output();
            #[cfg(not(any(unix, windows)))]
            let result: Result<std::process::Output, std::io::Error> = Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "exec_shell not supported on this platform",
            ));

            match result {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                    let exit_code = out.status.code().unwrap_or(-1) as i64;
                    Ok(ok(make_process_result(stdout, stderr, exit_code)))
                }
                Err(e) => Ok(err(format!("exec_shell: failed to spawn shell: {}", e))),
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
