use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

pub(crate) fn run(binary: &OsStr, certificate: &Path) -> crate::RResult<bool> {
    let output = Command::new(binary)
        .arg("-smt2")
        .arg(certificate)
        .output()
        .map_err(|error| format!("could not spawn z3: {error}"))?;
    // An UNSAT prefix cannot authenticate a failed or ambiguous solver run.
    if !output.status.success() || !output.stderr.is_empty() {
        return Err("z3 replay failed or emitted diagnostics".into());
    }
    let stdout = std::str::from_utf8(&output.stdout)
        .map_err(|_| "z3 replay returned invalid UTF-8".to_string())?;
    Ok(stdout.trim() == "unsat")
}
