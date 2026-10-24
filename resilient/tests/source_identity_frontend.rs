//! Frontend observations only: no source-origin or implementation-proof acceptance.
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
const SOURCES: &[(&str, &str)] = &[
    (
        "modules.rz",
        include_str!("fixtures/source_identity_frontend/modules.rz"),
    ),
    (
        "modules-library.rz",
        include_str!("fixtures/source_identity_frontend/modules-library.rz"),
    ),
    (
        "utf8.rz",
        include_str!("fixtures/source_identity_frontend/utf8.rz"),
    ),
    (
        "collision.rz",
        include_str!("fixtures/source_identity_frontend/collision.rz"),
    ),
    (
        "paths.rz",
        include_str!("fixtures/source_identity_frontend/paths.rz"),
    ),
    (
        "method.rz",
        include_str!("fixtures/source_identity_frontend/method.rz"),
    ),
    (
        "library.rz",
        include_str!("fixtures/source_identity_frontend/library.rz"),
    ),
    (
        "aliases.rz",
        include_str!("fixtures/source_identity_frontend/aliases.rz"),
    ),
    (
        "selective.rz",
        include_str!("fixtures/source_identity_frontend/selective.rz"),
    ),
    (
        "glob.rz",
        include_str!("fixtures/source_identity_frontend/glob.rz"),
    ),
    (
        "dedup.rz",
        include_str!("fixtures/source_identity_frontend/dedup.rz"),
    ),
    (
        "shadow.rz",
        include_str!("fixtures/source_identity_frontend/shadow.rz"),
    ),
    (
        "nested.rz",
        include_str!("fixtures/source_identity_frontend/nested.rz"),
    ),
    (
        "no_execution.rz",
        include_str!("fixtures/source_identity_frontend/no_execution.rz"),
    ),
];
const CATALOG: &str = include_str!("fixtures/source_identity_frontend/expected.json");

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
                    failure = Some("output-limit");
                }
            }
            Ok((index, Err(()))) => {
                eof[index] = true;
                failure = Some("pipe-read");
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

fn stage(result: &Captured, diagnostics: &Option<Value>) -> &'static str {
    if result.failure.is_some() {
        return "harness-failure";
    }
    if result.code == Some(0) {
        return "frontend-valid";
    }
    if result.code.is_none() {
        return "signal";
    }
    if let Some(Value::Array(items)) = diagnostics {
        if items.iter().any(|item| item["code"] == "parse") {
            return "parse-rejected";
        }
        if items.iter().any(|item| item["code"] == "typecheck") {
            return "type-rejected";
        }
        return "other-structured-rejection";
    }
    if result.code == Some(2) {
        "input-or-cli-unavailable"
    } else {
        "import-or-unstructured-rejection"
    }
}

#[test]
fn frozen_source_catalog_frontend_observations_only() {
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_rz"));
    let binary_sha = hash(&fs::read(&binary).expect("binary bytes"));
    let version = bounded(Command::new(&binary).arg("--version"));
    let version_record = json!({"schema":"resilient-source-identity-frontend-version/v1","binary_sha256":binary_sha,"status":version.code,"harness_failure":version.failure,"stdout":String::from_utf8_lossy(&version.stdout),"stderr":String::from_utf8_lossy(&version.stderr)});
    let mut version_line = b"SOURCE_IDENTITY_FRONTEND_VERSION ".to_vec();
    version_line.extend(serde_json::to_vec(&version_record).expect("version JSON"));
    version_line.push(b'\n');
    std::io::stdout()
        .lock()
        .write_all(&version_line)
        .expect("raw version record");
    assert!(
        version.failure.is_none() && version.code == Some(0),
        "version unavailable"
    );
    assert_eq!(
        hash(CATALOG.as_bytes()),
        "2471958d652cc5aa8bb92e9ae800d99fc4b4ac6bdc222b89a131d8d0ba55ef51"
    );
    let catalog: Value = serde_json::from_str(CATALOG).expect("frozen catalog JSON");
    let dir = std::env::temp_dir().join(format!(
        "res_source_identity_frontend_{}",
        std::process::id()
    ));
    fs::create_dir(&dir).expect("fresh fixture directory");
    for (path, source) in SOURCES {
        fs::write(dir.join(path), source).expect("materialize source");
    }
    for file in catalog["files"].as_array().expect("catalog files") {
        let bytes =
            fs::read(dir.join(file["path"].as_str().expect("file path"))).expect("catalog file");
        assert_eq!(hash(&bytes), file["sha256"].as_str().expect("file SHA"));
        assert_eq!(
            bytes.len() as u64,
            file["bytes"].as_u64().expect("file size")
        );
    }
    let files: Vec<_> = SOURCES.iter().map(|(path, source)| json!({"path":path,"bytes":source.len(),"sha256":hash(source.as_bytes()),"source":source})).collect();
    let mut failures = Vec::new();
    for (path, _) in SOURCES {
        let args = [
            "check",
            *path,
            "--emit-diagnostics-json",
            "--verifier-timeout-ms",
            "2000",
        ];
        let result = bounded(Command::new(&binary).args(args).current_dir(&dir));
        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);
        let diagnostics = serde_json::from_slice::<Value>(&result.stdout).ok();
        let role = catalog["files"]
            .as_array()
            .expect("files")
            .iter()
            .find(|file| file["path"] == *path)
            .map(|file| file["role"].clone())
            .unwrap_or(json!("new-no-execution-control"));
        let record = json!({"schema":"resilient-source-identity-frontend-observation/v1","claim":"frontend observation only; no runtime, origin-mapping, proof or inventory-completeness credit","entry":path,"role":role,"argv":args,"files":files,"catalog_sha256":hash(CATALOG.as_bytes()),"driver_sha256":hash(include_bytes!("source_identity_frontend.rs")),"binary_path":binary,"binary_sha256":binary_sha,"version_status":version.code,"version_stdout":String::from_utf8_lossy(&version.stdout),"version_stderr":String::from_utf8_lossy(&version.stderr),"features":{"z3":cfg!(feature="z3"),"lsp":cfg!(feature="lsp"),"jit":cfg!(feature="jit")},"overflow_environment":std::env::var("RESILIENT_OVERFLOW_MODE").ok(),"stage":stage(&result,&diagnostics),"status":result.code,"harness_failure":result.failure,"stdout":stdout,"stderr":stderr,"diagnostics":diagnostics});
        let mut line = serde_json::to_vec(&record).expect("record JSON");
        line.splice(
            0..0,
            b"SOURCE_IDENTITY_FRONTEND_OBSERVATION ".iter().copied(),
        );
        line.push(b'\n');
        std::io::stdout()
            .lock()
            .write_all(&line)
            .expect("raw observation record");
        if result.failure.is_some() || result.code.is_none() || stderr.contains("panicked at") {
            failures.push(format!("{path}: harness/panic"));
        }
        if (*path == "utf8.rz" || *path == "no_execution.rz") && result.code != Some(0) {
            failures.push(format!("{path}: valid control rejected"));
        }
        if stdout.contains("SOURCE_IDENTITY_MUST_NOT_EXECUTE")
            || stderr.contains("SOURCE_IDENTITY_MUST_NOT_EXECUTE")
        {
            failures.push(format!("{path}: executed sentinel"));
        }
    }
    fs::remove_dir_all(&dir).expect("remove owned fixture directory");
    assert!(
        failures.is_empty(),
        "observation harness/control failures: {failures:?}"
    );
}
