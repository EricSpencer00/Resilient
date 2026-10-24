//! Physical-origin command requirements; command absence is feature absence, not semantic red.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
const LIMIT: usize = 1024 * 1024;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
struct Captured {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    failure: Option<&'static str>,
}

fn bounded(command: &mut Command) -> Captured {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rz");
    let streams: Vec<Box<dyn Read + Send>> = vec![
        Box::new(child.stdout.take().expect("stdout pipe")),
        Box::new(child.stderr.take().expect("stderr pipe")),
    ];
    let (tx, rx) = mpsc::sync_channel(8);
    let readers: Vec<_> = streams
        .into_iter()
        .enumerate()
        .map(|(index, mut pipe)| {
            let tx = tx.clone();
            thread::spawn(move || {
                let mut buf = [0; 4096];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) => {
                            let _ = tx.send((index, Ok(Vec::new())));
                            break;
                        }
                        Ok(n) => {
                            if tx.send((index, Ok(buf[..n].to_vec()))).is_err() {
                                break;
                            }
                        }
                        Err(_) => {
                            let _ = tx.send((index, Err(())));
                            break;
                        }
                    }
                }
            })
        })
        .collect();
    drop(tx);
    let start = Instant::now();
    let mut stopped = None;
    let mut failure = None;
    let mut eof = [false; 2];
    let mut output = [Vec::new(), Vec::new()];
    let mut status = None;
    loop {
        if failure.is_none() && start.elapsed() > Duration::from_secs(30) {
            failure = Some("timeout");
        }
        if failure.is_some() && stopped.is_none() {
            let _ = child.kill();
            status = Some(child.wait().expect("reap killed rz"));
            stopped = Some(Instant::now());
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok((index, Ok(bytes))) => {
                if bytes.is_empty() {
                    eof[index] = true;
                } else if output[index].len() + bytes.len() <= LIMIT {
                    output[index].extend(bytes);
                } else {
                    failure.get_or_insert("output-limit");
                }
            }
            Ok((index, Err(()))) => {
                eof[index] = true;
                failure.get_or_insert("pipe-read");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eof = [true; 2];
                thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().expect("poll rz");
        }
        if status.is_some() && eof.iter().all(|done| *done) {
            break;
        }
        // A descendant retaining a pipe cannot make the harness wait forever.
        if stopped.is_some_and(|when| when.elapsed() > Duration::from_secs(2)) {
            break;
        }
    }
    drop(rx);
    for reader in readers {
        if reader.is_finished() {
            reader.join().expect("capture reader");
        }
    }
    let status = status.unwrap_or_else(|| child.wait().expect("reap rz"));
    Captured {
        code: status.code(),
        stdout: std::mem::take(&mut output[0]),
        stderr: std::mem::take(&mut output[1]),
        failure,
    }
}

fn invoke(path: &std::path::Path, key: &str, profile: &str) -> (Captured, Option<Value>) {
    let args = [
        "source-origins",
        path.to_str().expect("fixture path"),
        "--source-key",
        key,
        "--profile",
        profile,
    ];
    invoke_args(path, &args)
}
fn bounded_fixture_hash(path: &std::path::Path) -> (Option<String>, &'static str) {
    const MAX: u64 = 1024 * 1024 + 1;
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return (None, "unavailable");
    };
    if !metadata.is_file() {
        return (None, "symlink-or-nonregular-no-read");
    }
    if metadata.len() > MAX {
        return (None, "oversized-no-read");
    }
    let Ok(file) = fs::File::open(path) else {
        return (None, "open-unavailable");
    };
    let Ok(metadata) = file.metadata() else {
        return (None, "handle-metadata-unavailable");
    };
    if !metadata.is_file() || metadata.len() > MAX {
        return (None, "handle-nonregular-or-oversized");
    }
    let mut bytes = Vec::new();
    if file.take(MAX + 1).read_to_end(&mut bytes).is_err() {
        return (None, "read-unavailable");
    }
    if bytes.len() as u64 > MAX {
        return (None, "grew-beyond-bound");
    }
    (Some(hash(&bytes)), "stable-regular-bounded-read")
}
fn invoke_args(path: &std::path::Path, args: &[&str]) -> (Captured, Option<Value>) {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_rz"));
    let output = bounded(Command::new(&binary).args(args));
    let parsed = serde_json::from_slice::<Value>(&output.stdout).ok();
    let known_report = parsed
        .as_ref()
        .is_some_and(|v| v["schema"] == "resilient-physical-origins/v1");
    let (input_sha, input_hash_reason) = bounded_fixture_hash(path);
    let record = json!({"schema":"resilient-source-origins-cli-requirement/v1","claim":"physical metadata only; no runtime/typecheck/solver/binding/proof credit","argv":args,"input_sha256":input_sha,"input_hash_reason":input_hash_reason,"binary_sha256":hash(&fs::read(&binary).expect("binary bytes")),"driver_sha256":hash(include_bytes!("source_origins_cli.rs")),"features":{"z3":cfg!(feature="z3"),"lsp":cfg!(feature="lsp"),"jit":cfg!(feature="jit")},"status":output.code,"harness_failure":output.failure,"protocol":if known_report {"recognized-origin-report"} else {"feature-absence-or-unrecognized-protocol"},"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"report":parsed});
    let mut line = b"SOURCE_ORIGINS_CLI_REQUIREMENT ".to_vec();
    line.extend(serde_json::to_vec(&record).expect("record JSON"));
    line.push(b'\n');
    std::io::stdout()
        .lock()
        .write_all(&line)
        .expect("raw record");
    (output, parsed)
}
fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source_origins_cli")
}
#[test]
fn canonical_parser_origin_vectors_and_zero_clause_declarations() {
    let vectors: Value = serde_json::from_str(include_str!(
        "fixtures/source_origins_cli/expected-vectors.json"
    ))
    .expect("manual vectors");
    let mut errors = Vec::new();
    for vector in vectors["vectors"].as_array().expect("vectors") {
        let path = fixtures().join(vector["fixture"].as_str().expect("fixture"));
        assert_eq!(
            hash(&fs::read(&path).expect("source")),
            vector["source_sha256"].as_str().expect("source hash")
        );
        let (raw, report) = invoke(
            &path,
            vector["source_key"].as_str().expect("logical key"),
            "physical-origins-v1",
        );
        if raw.failure.is_some()
            || raw.stdout.len() > 524288
            || raw.code != Some(0)
            || report.as_ref() != Some(&vector["expected"])
        {
            errors.push(vector["fixture"].clone());
        }
        assert!(
            !String::from_utf8_lossy(&raw.stdout)
                .lines()
                .any(|line| line == "SOURCE_ORIGINS_MUST_NOT_EXECUTE"),
            "origin command executed source"
        );
    }
    assert!(
        errors.is_empty(),
        "feature absence or origin requirement mismatch (not compiler-semantic baseline): {errors:?}"
    );
}
fn no_accepted_origins(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().all(|(key, value)| {
            ![
                "declarations",
                "sites",
                "declaration_id",
                "site_id",
                "identity",
                "origins",
            ]
            .contains(&key.as_str())
                && no_accepted_origins(value)
        }),
        Value::Array(items) => items.iter().all(no_accepted_origins),
        _ => true,
    }
}
fn rejected(
    raw: &Captured,
    report: &Option<Value>,
    key: Option<&str>,
    profile: Option<&str>,
    status: i32,
    code: &str,
    limit: Option<&str>,
) -> bool {
    raw.failure.is_none()
        && raw.stdout.len() <= 524288
        && raw.code == Some(status)
        && report.as_ref().is_some_and(|v| {
            v["schema"] == "resilient-physical-origins/v1"
                && v.get("profile") == Some(&json!(profile))
                && v.get("source_key") == Some(&json!(key))
                && v["key_authority"] == "caller-supplied-not-authenticated-path"
                && v.get("file_sha256").is_some()
                && v.get("bytes").is_some()
                && v["disposition"] == "rejected"
                && v["error"]["code"] == code
                && limit.is_none_or(|l| v["error"]["limit"] == l)
                && no_accepted_origins(v)
        })
}
#[test]
fn whole_unit_parse_recovery_and_unsupported_reject_without_partial_origins() {
    let cases = [
        ("recovery.rz", "parse-error"),
        ("recovery-requires.rz", "parse-error"),
        ("unsupported.rz", "unsupported-grammar"),
        ("execution-entry.rz", "unsupported-grammar"),
    ];
    let mut errors = Vec::new();
    for (file, code) in cases {
        let key = format!("fixtures/{file}");
        let (raw, report) = invoke(&fixtures().join(file), &key, "physical-origins-v1");
        assert!(
            !String::from_utf8_lossy(&raw.stdout)
                .lines()
                .any(|line| line == "SOURCE_ORIGINS_MUST_NOT_EXECUTE"),
            "rejected entry executed"
        );
        if !rejected(
            &raw,
            &report,
            Some(&key),
            Some("physical-origins-v1"),
            1,
            code,
            None,
        ) {
            errors.push(file);
        }
    }
    assert!(
        errors.is_empty(),
        "feature absence or whole-unit rejection-protocol mismatch: {errors:?}"
    );
}
#[test]
fn external_profile_and_canonical_caller_key_are_required() {
    let path = fixtures().join("utf8.rz");
    let mut errors = Vec::new();
    for (key, profile, code) in [
        ("../escape.rz", "physical-origins-v1", "invalid-source-key"),
        ("/absolute.rz", "physical-origins-v1", "invalid-source-key"),
        ("a//b.rz", "physical-origins-v1", "invalid-source-key"),
        ("a\\b.rz", "physical-origins-v1", "invalid-source-key"),
        ("a:b.rz", "physical-origins-v1", "invalid-source-key"),
        ("valid.rz", "legacy", "unsupported-profile"),
        (
            "decl-sha256:x.rz",
            "physical-origins-v1",
            "invalid-source-key",
        ),
        ("valid.rz", "site-sha256:x", "unsupported-profile"),
    ] {
        let (raw, report) = invoke(&path, key, profile);
        if !rejected(&raw, &report, Some(key), Some(profile), 2, code, None) {
            errors.push((key, profile));
        }
    }
    assert!(
        errors.is_empty(),
        "feature absence or external-policy mismatch: {errors:?}"
    );
}
#[test]
fn missing_duplicate_unknown_and_trailing_policy_arguments_reject() {
    let path = fixtures().join("utf8.rz");
    let file = path.to_str().expect("path");
    let cases: Vec<(Vec<&str>, Option<&str>, Option<&str>)> = vec![
        (vec!["source-origins", file], None, None),
        (
            vec!["source-origins", file, "--source-key", "valid.rz"],
            Some("valid.rz"),
            None,
        ),
        (
            vec!["source-origins", file, "--profile", "physical-origins-v1"],
            None,
            Some("physical-origins-v1"),
        ),
        (
            vec![
                "source-origins",
                file,
                "--source-key",
                "valid.rz",
                "--source-key",
                "other.rz",
                "--profile",
                "physical-origins-v1",
            ],
            None,
            Some("physical-origins-v1"),
        ),
        (
            vec![
                "source-origins",
                file,
                "--source-key",
                "valid.rz",
                "--profile",
                "physical-origins-v1",
                "--profile",
                "physical-origins-v1",
            ],
            Some("valid.rz"),
            None,
        ),
        (
            vec![
                "source-origins",
                file,
                "--source-key",
                "valid.rz",
                "--profile",
                "physical-origins-v1",
                "--unknown",
            ],
            Some("valid.rz"),
            Some("physical-origins-v1"),
        ),
        (
            vec![
                "source-origins",
                file,
                "--source-key",
                "valid.rz",
                "--profile",
                "physical-origins-v1",
                "extra.rz",
            ],
            Some("valid.rz"),
            Some("physical-origins-v1"),
        ),
    ];
    let mut errors = Vec::new();
    for (index, (args, key, profile)) in cases.iter().enumerate() {
        let (raw, report) = invoke_args(&path, args);
        if !rejected(&raw, &report, *key, *profile, 2, "invalid-cli", None) {
            errors.push(index);
        }
    }
    assert!(
        errors.is_empty(),
        "feature absence or explicit policy protocol mismatch: {errors:?}"
    );
}
#[test]
fn metadata_limits_have_preparse_precedence_and_no_partial_report() {
    let sources = [
        ("source-bytes", vec![b' '; 1024 * 1024 + 1]),
        (
            "tokens",
            format!("fn many() {{ {} }}", "0;".repeat(32769)).into_bytes(),
        ),
        (
            "delimiter-depth",
            format!(
                "{}{}{}",
                "mod m {".repeat(129),
                "fn f() {}",
                "}".repeat(129)
            )
            .into_bytes(),
        ),
        (
            "function-keywords",
            (0..4097)
                .map(|i| format!("fn f{i}() {{}}\n"))
                .collect::<String>()
                .into_bytes(),
        ),
        (
            "contract-keywords",
            format!(
                "fn f(int x) -> int {} {{return x;}}",
                "requires x >= 0 ".repeat(8193)
            )
            .into_bytes(),
        ),
        (
            "output-bytes",
            (0..2000)
                .map(|i| format!("fn f{i}() {{}}\n"))
                .collect::<String>()
                .into_bytes(),
        ),
    ];
    let mut errors = Vec::new();
    for (name, source) in sources {
        let dir = std::env::temp_dir().join(format!("source_origin_{name}_{}", std::process::id()));
        fs::create_dir(&dir).expect("fresh directory");
        let path = dir.join("input.rz");
        fs::write(&path, source).expect("generated limit source");
        let (raw, report) = invoke(&path, "fixtures/input.rz", "physical-origins-v1");
        fs::remove_dir_all(dir).expect("remove owned source");
        if !rejected(
            &raw,
            &report,
            Some("fixtures/input.rz"),
            Some("physical-origins-v1"),
            1,
            "resource-limit",
            Some(name),
        ) {
            errors.push(name);
        }
    }
    assert!(
        errors.is_empty(),
        "feature absence or targeted limit protocol mismatch: {errors:?}"
    );
}
#[test]
fn invalid_utf8_and_stable_nonregular_inputs_reject() {
    let dir = std::env::temp_dir().join(format!("source_origin_inputs_{}", std::process::id()));
    fs::create_dir(&dir).expect("fresh directory");
    let invalid = dir.join("invalid.rz");
    fs::write(&invalid, [0xff, 0xfe]).expect("invalid UTF8");
    let mut errors = Vec::new();
    for (path, code) in [(&invalid, "invalid-utf8"), (&dir, "input-not-regular")] {
        let (raw, report) = invoke(path, "fixtures/input.rz", "physical-origins-v1");
        if !rejected(
            &raw,
            &report,
            Some("fixtures/input.rz"),
            Some("physical-origins-v1"),
            2,
            code,
            None,
        ) {
            errors.push(code);
        }
    }
    #[cfg(unix)]
    {
        let link = dir.join("link.rz");
        std::os::unix::fs::symlink(&invalid, &link).expect("owned symlink");
        let (raw, report) = invoke(&link, "fixtures/input.rz", "physical-origins-v1");
        if !rejected(
            &raw,
            &report,
            Some("fixtures/input.rz"),
            Some("physical-origins-v1"),
            2,
            "input-symlink",
            None,
        ) {
            errors.push("input-symlink");
        }
        let fifo = dir.join("fifo.rz");
        let made = bounded(Command::new("mkfifo").arg(&fifo));
        assert!(
            made.failure.is_none() && made.code == Some(0),
            "hosted Unix mkfifo fixture unavailable"
        );
        let (raw, report) = invoke(&fifo, "fixtures/input.rz", "physical-origins-v1");
        if !rejected(
            &raw,
            &report,
            Some("fixtures/input.rz"),
            Some("physical-origins-v1"),
            2,
            "input-not-regular",
            None,
        ) {
            errors.push("stable-fifo");
        }
    }
    fs::remove_dir_all(dir).expect("remove owned input fixtures");
    assert!(
        errors.is_empty(),
        "feature absence or stable input policy mismatch: {errors:?}"
    );
}
