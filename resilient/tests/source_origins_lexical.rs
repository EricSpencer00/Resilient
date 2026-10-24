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
    let record = json!({"schema":"resilient-source-origins-lexical-requirement/v1","claim":"physical metadata only; no runtime/typecheck/solver/binding/proof credit","argv":args,"input_sha256":input_sha,"input_hash_reason":input_hash_reason,"binary_sha256":hash(&fs::read(&binary).expect("binary bytes")),"driver_sha256":hash(include_bytes!("source_origins_lexical.rs")),"catalog_sha256":hash(include_bytes!("fixtures/source_origins_lexical/expected.json")),"features":{"z3":cfg!(feature="z3"),"lsp":cfg!(feature="lsp"),"jit":cfg!(feature="jit")},"status":output.code,"harness_failure":output.failure,"protocol":if known_report {"recognized-origin-report"} else {"feature-absence-or-unrecognized-protocol"},"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr),"report":parsed});
    let mut line = b"SOURCE_ORIGINS_LEXICAL_REQUIREMENT ".to_vec();
    line.extend(serde_json::to_vec(&record).expect("record JSON"));
    line.push(b'\n');
    std::io::stdout()
        .lock()
        .write_all(&line)
        .expect("raw record");
    (output, parsed)
}
fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/source_origins_lexical")
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
fn physical_origin_lexical_completion_and_eof_requirements() {
    let catalog: Value = serde_json::from_str(include_str!(
        "fixtures/source_origins_lexical/expected.json"
    ))
    .expect("manual lexical vectors");
    let mut errors = Vec::new();
    for vector in catalog["vectors"].as_array().expect("vectors") {
        let file = vector["file"].as_str().expect("file");
        let path = fixtures().join(file);
        assert_eq!(
            hash(&fs::read(&path).expect("frozen regular fixture")),
            vector["sha256"].as_str().expect("sourceSHA")
        );
        let key = format!("fixtures/{file}");
        let (raw, report) = invoke(&path, &key, "physical-origins-v1");
        let valid = if vector["expected_disposition"] == "accepted" {
            raw.failure.is_none()
                && raw.code == Some(0)
                && raw.stdout.len() <= 524288
                && report.as_ref() == Some(&vector["expected_report"])
        } else {
            rejected(
                &raw,
                &report,
                Some(&key),
                Some("physical-origins-v1"),
                1,
                "parse-error",
                None,
            ) && report
                .as_ref()
                .is_some_and(|v| v["error"]["lexical_reason"] == vector["lexical_reason"])
        };
        if !valid {
            errors.push(file.to_owned());
        }
    }
    assert!(
        errors.is_empty(),
        "feature absence or lexical-origin requirement mismatch (not normal-parser semantic red): {errors:?}"
    );
}
