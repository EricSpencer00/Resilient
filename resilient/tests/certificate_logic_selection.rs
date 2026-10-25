//! The unchanged observed nonlinear call must survive signed public replay.
#![cfg(feature = "z3")]
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
const CAPTURE_LIMIT: usize = 1_048_576;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("nonregular public artifact"));
    }
    let mut digest = Sha256::new();
    let mut bytes = [0; 8192];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn read_public(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(std::io::Error::other("public artifact bound/type"));
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("nonregular opened artifact"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other("public artifact read bound"));
    }
    Ok(bytes)
}
struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn emit_record(record: Value) {
    let mut line = b"CERTIFICATE_LOGIC_CONTROL ".to_vec();
    line.extend(serde_json::to_vec(&record).unwrap());
    line.push(b'\n');
    std::io::stdout().lock().write_all(&line).unwrap();
}
fn captured(output: &Captured) -> Value {
    json!({"status":output.code,"harness_failure":output.failure,"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)})
}
struct Captured {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    failure: Option<&'static str>,
}

fn run(command: &mut Command) -> Captured {
    let mut child = match command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return Captured {
                code: None,
                stdout: Vec::new(),
                stderr: error.to_string().into_bytes(),
                failure: Some("spawn-failed"),
            };
        }
    };
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
                } else if output[index].len() + bytes.len() <= CAPTURE_LIMIT {
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

#[test]
fn unchanged_observed_nonlinear_call_has_compatible_signed_logic() {
    let source = include_bytes!("fixtures/certificate_logic/observed_nonlinear_call.rz");
    assert_eq!(
        hash(source),
        "8ec51e99a5202e0d9a2825957ad9839082c0e7d096d52ea0570cf4e23636475a"
    );
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let owned =
        std::env::temp_dir().join(format!("certificate_logic_{}_{nonce}", std::process::id()));
    // Establish ownership before arming cleanup: an AlreadyExists error must
    // never remove a pre-existing directory.
    fs::create_dir(&owned).unwrap();
    let fixture = Fixture(owned);
    let root = &fixture.0;
    fs::write(root.join("program.rz"), source).unwrap();
    let key = SigningKey::generate(&mut rand_core::OsRng);
    for (name, label, bytes) in [
        ("private.pem", "PRIVATE", key.to_bytes()),
        ("public.pem", "PUBLIC", key.verifying_key().to_bytes()),
    ] {
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        fs::write(
            root.join(name),
            format!(
                "-----BEGIN ED25519 {label} KEY-----\n{hex}\n-----END ED25519 {label} KEY-----\n"
            ),
        )
        .unwrap();
    }
    let binary = Path::new(env!("CARGO_BIN_EXE_rz"));
    let solver_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("z3"))
            .find(|file| file.is_file())
    });
    let solver =
        run(Command::new(solver_path.as_deref().unwrap_or(Path::new("z3"))).arg("-version"));
    let identity = json!({"schema":"resilient-certificate-logic-control/v2","source_sha256":hash(source),"source_utf8":String::from_utf8_lossy(source),"driver_sha256":hash(include_bytes!("certificate_logic_selection.rs")),"binary_sha256":hash_file(binary).unwrap(),"public_key_pem":String::from_utf8(read_public(&root.join("public.pem"),512).unwrap()).unwrap(),"solver_path":solver_path,"solver_sha256":solver_path.as_deref().and_then(|p|hash_file(p).ok()),"solver_version":captured(&solver),"limitations":"direct-child cleanup only; descendant process-group adversaries excluded; solver/compiler shared-library closure not captured; no separate frontend acceptance claim","claim":"signed replay and compatible serialization logic only; no source-to-query completeness or machine-arithmetic equivalence"});
    emit_record(json!({"stage":"identity","identity":identity}));
    assert!(
        solver.failure.is_none() && solver.code == Some(0),
        "solver availability/version: {}",
        captured(&solver)
    );
    let export = run(Command::new(binary)
        .args(["--no-cache", "-t", "--emit-certificate"])
        .arg(root.join("certs"))
        .arg("--sign-cert")
        .arg(root.join("private.pem"))
        .arg(root.join("program.rz")));
    emit_record(
        json!({"stage":"signed-export","identity":identity,"result":captured(&export),"argv_shape":["--no-cache","-t","--emit-certificate","OWNED_CERT_DIR","--sign-cert","OWNED_EPHEMERAL_PRIVATE_KEY_PATH","UNCHANGED_SOURCE"]}),
    );
    assert!(
        export.failure.is_none() && export.code == Some(0),
        "signed emit: {}",
        captured(&export)
    );
    let manifest_bytes = read_public(&root.join("certs/manifest.json"), 1_048_576);
    emit_record(
        json!({"stage":"manifest-read","identity":identity,"read_error":manifest_bytes.as_ref().err().map(ToString::to_string),"raw_utf8":manifest_bytes.as_ref().ok().map(|bytes|String::from_utf8_lossy(bytes)),"actual_sha256":manifest_bytes.as_ref().ok().map(|bytes|hash(bytes))}),
    );
    let manifest: Value = serde_json::from_slice(&manifest_bytes.unwrap()).unwrap();
    let obligations = manifest["obligations"].as_array().unwrap();
    assert_eq!(
        obligations.len(),
        2,
        "both unchanged declaration/call-site controls must be retained"
    );
    let manifest_signature = read_public(&root.join("certs/manifest.sig"), 512);
    emit_record(
        json!({"stage":"manifest-signature","identity":identity,"read_error":manifest_signature.as_ref().err().map(ToString::to_string),"raw_utf8":manifest_signature.as_ref().ok().map(|bytes|String::from_utf8_lossy(bytes))}),
    );
    let manifest_signature = String::from_utf8(manifest_signature.unwrap()).unwrap();
    let mut queries = Vec::new();
    let mut remaining = 1_048_576u64;
    for obligation in obligations {
        let file = obligation["cert"].as_str().unwrap();
        assert!(!file.contains('/') && !file.contains('\\') && file.ends_with(".smt2"));
        let bytes = read_public(&root.join("certs").join(file), remaining);
        emit_record(
            json!({"stage":"query-read","identity":identity,"manifest_entry":obligation,"read_error":bytes.as_ref().err().map(ToString::to_string),"raw_utf8":bytes.as_ref().ok().map(|bytes|String::from_utf8_lossy(bytes)),"actual_sha256":bytes.as_ref().ok().map(|bytes|hash(bytes))}),
        );
        let bytes = bytes.unwrap();
        remaining -= bytes.len() as u64;
        let query = String::from_utf8(bytes.clone()).unwrap();
        queries
            .push(json!({"manifest_entry":obligation,"actual_sha256":hash(&bytes),"query":query}));
    }
    let replay = run(Command::new(binary)
        .arg("verify-all")
        .arg(root.join("certs"))
        .arg("--source-root")
        .arg(root)
        .arg("--pubkey")
        .arg(root.join("public.pem"))
        .args(["--require-manifest-signature", "--z3"]));
    emit_record(
        json!({"stage":"strict-signed-replay","identity":identity,"manifest":manifest,"manifest_signature_hex":manifest_signature,"queries":queries,"result":captured(&replay),"argv_shape":["verify-all","OWNED_CERT_DIR","--source-root","OWNED_SOURCE_ROOT","--pubkey","OWNED_PUBLIC_KEY_PATH","--require-manifest-signature","--z3"]}),
    );
    assert_eq!(
        obligations.len(),
        2,
        "both unchanged declaration/call-site controls must be retained"
    );
    assert!(obligations.iter().any(|entry| entry["kind"] == "decl"));
    assert!(
        obligations
            .iter()
            .any(|entry| entry["kind"] == "callsite_requires")
    );
    assert!(
        replay.failure.is_none() && replay.code == Some(0),
        "strict signed replay: {}",
        captured(&replay)
    );
    for query in queries {
        assert_eq!(query["actual_sha256"], query["manifest_entry"]["sha256"]);
        assert!(query["manifest_entry"]["sig"].is_string());
        assert!(
            query["query"]
                .as_str()
                .unwrap()
                .contains("(set-logic AUFNIRA)"),
            "{query}"
        );
    }
}
