//! Strict proof requests must reach file execution rather than an early-exit mode.
pub(crate) fn validate(args: &mut Vec<String>) -> Result<bool, String> {
    const FLAG: &str = "--deny-unproven-invariants";
    if !args.iter().any(|arg| arg == FLAG) {
        return Ok(false);
    }
    let switches = [
        FLAG,
        "--typecheck",
        "-t",
        "--typecheck-strict",
        "--audit",
        "--verbose",
        "--vm",
        "--jit",
        "--no-cache",
        "--warn-unverified",
        "--no-warn-unverified",
        "--explain-effects",
        "--watch",
        "--deny-unproven-bounds",
        "--strict-termination",
        "--safety-critical",
        "--panic-on-fault",
        "--no-panic-on-fault",
    ];
    let values = [
        "--verifier-timeout-ms",
        "--seed",
        "--emit-certificate",
        "--sign-cert",
        "--emit-contract-certificate",
        "--emit-live-log",
        "--persistent-proof-cache",
        "--z3-theory",
        "--feature",
        "--target",
        "--cfg",
    ];
    let mut files = 0;
    let mut index = 1;
    while index < args.len() {
        let arg = &args[index];
        if switches.contains(&arg.as_str()) {
        } else if values.contains(&arg.as_str()) {
            index += 1;
            if index >= args.len() || args[index].starts_with('-') {
                return Err(format!("{FLAG}: {arg} requires a value"));
            }
        } else if arg
            .split_once('=')
            .is_some_and(|(key, value)| values.contains(&key) && !value.is_empty())
        {
        } else if arg.starts_with('-') {
            return Err(format!("{FLAG}: incompatible or unsupported option {arg}"));
        } else {
            files += 1;
        }
        index += 1;
    }
    if files != 1 {
        return Err(format!(
            "{FLAG} requires normal execution of exactly one source file; subcommands and interactive modes are unsupported"
        ));
    }
    // Keep first-argument subcommand dispatch from consuming the proof request.
    args.retain(|arg| arg != FLAG);
    args.insert(1, FLAG.to_string());
    Ok(true)
}
