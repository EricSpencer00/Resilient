//! NEW diagnostic requirements: a successful export must remain a replayable complete bundle.
//! Private draft; no existing tests, production, or workflow edits.
#![cfg(all(feature = "z3", target_os = "linux"))]

use ed25519_dalek::SigningKey;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Capture {
    code: Option<i32>,
    success: bool,
    timed_out: bool,
    stdout: String,
    stderr: String,
    harness_failure: Option<&'static str>,
}
fn command_at(program: &Path, argv: &[String]) -> Capture {
    const LIMIT: usize = 262_144;
    let spawned = Command::new(program)
        .args(argv)
        .env_remove("RESILIENT_OVERFLOW_MODE")
        .env("Z3_BINARY", "z3")
        .env("RZ_SOLVER_TIMEOUT_MS", "5000")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => {
            return Capture {
                code: None,
                success: false,
                timed_out: false,
                stdout: String::new(),
                stderr: error.to_string(),
                harness_failure: Some("spawn"),
            };
        }
    };
    let streams: Vec<Box<dyn Read + Send>> = vec![
        Box::new(child.stdout.take().unwrap()),
        Box::new(child.stderr.take().unwrap()),
    ];
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let readers: Vec<_> = streams
        .into_iter()
        .enumerate()
        .map(|(index, mut pipe)| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut bytes = [0; 4096];
                loop {
                    match pipe.read(&mut bytes) {
                        Ok(0) => {
                            let _ = tx.send((index, Ok(Vec::new())));
                            break;
                        }
                        Ok(n) => {
                            if tx.send((index, Ok(bytes[..n].to_vec()))).is_err() {
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
        if failure.is_none() && start.elapsed() > Duration::from_secs(25) {
            failure = Some("timeout");
        }
        if failure.is_some() && stopped.is_none() {
            // Keep process-group cleanup, but never join a reader whose descendant holds a pipe.
            if let Ok(mut killer) = Command::new("/bin/kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .spawn()
            {
                let until = Instant::now() + Duration::from_secs(1);
                while killer.try_wait().ok().flatten().is_none() && Instant::now() < until {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = killer.kill();
                let _ = killer.wait();
            }
            let _ = child.kill();
            status = Some(child.wait().expect("reap stopped process"));
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
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                eof = [true; 2];
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
        }
        if status.is_none() {
            status = child.try_wait().expect("poll child");
        }
        if status.is_some() && eof.iter().all(|x| *x) {
            break;
        }
        if stopped.is_some_and(|when: Instant| when.elapsed() > Duration::from_secs(2)) {
            break;
        }
    }
    drop(rx);
    for reader in readers {
        if reader.is_finished() {
            let _ = reader.join();
        }
    }
    let status = status.unwrap_or_else(|| child.wait().expect("reap child"));
    Capture {
        code: status.code(),
        success: status.success(),
        timed_out: failure == Some("timeout"),
        stdout: String::from_utf8_lossy(&output[0]).into_owned(),
        stderr: String::from_utf8_lossy(&output[1]).into_owned(),
        harness_failure: failure,
    }
}

fn bounded_regular(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| format!("metadata: {e}"))?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("nonregular/symlink or oversized artifact".into());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let opened = file
        .metadata()
        .map_err(|e| format!("handle metadata: {e}"))?;
    if !opened.is_file() || opened.len() > limit as u64 {
        return Err("nonregular or oversized opened artifact".into());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read: {e}"))?;
    if bytes.len() > limit {
        return Err("artifact grew beyond bound".into());
    }
    Ok(bytes)
}
fn artifact_snapshot(root: &Path) -> Result<BTreeMap<String, Value>, String> {
    const FILE_LIMIT: usize = 262_144;
    const TOTAL_LIMIT: usize = 1_048_576;
    const COUNT_LIMIT: usize = 32;
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("artifact root: {e}")),
    };
    if !metadata.is_dir() {
        return Err("artifact root is symlink or non-directory".into());
    }
    let mut files = BTreeMap::new();
    let mut total = 0;
    for entry in std::fs::read_dir(root).map_err(|e| format!("artifact listing: {e}"))? {
        if files.len() >= COUNT_LIMIT {
            return Err("artifact file-count exceeded".into());
        }
        let path = entry.map_err(|e| format!("artifact entry: {e}"))?.path();
        let bytes = bounded_regular(&path, FILE_LIMIT.min(TOTAL_LIMIT - total))?;
        total += bytes.len();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("nonUTF8 artifact name")?
            .to_string();
        files.insert(
            name,
            json!({"sha256":hash(&bytes),"bytes":bytes.len(),"utf8":String::from_utf8(bytes).ok()}),
        );
    }
    Ok(files)
}

fn hash_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut digest = Sha256::new();
    let mut block = [0u8; 65536];
    loop {
        let n = file.read(&mut block).ok()?;
        if n == 0 {
            break;
        }
        digest.update(&block[..n]);
    }
    Some(format!("{:x}", digest.finalize()))
}
fn identities() -> &'static Value {
    static IDENTITIES: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    IDENTITIES.get_or_init(|| {
        let compiler=Path::new(env!("CARGO_BIN_EXE_rz"));
        let version=command_at(compiler, &["--version".into(),"--verbose".into()]);
        let compiler_hash=hash_file(compiler);
        let solver=std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).map(|p|p.join("z3")).find(|p|p.is_file()).and_then(|p|p.canonicalize().ok());
        let Some(solver)=solver else{return json!({"valid":false,"identity_failure":"solver-unavailable","compiler_version":version.stdout,"compiler_version_code":version.code,"compiler_version_stderr":version.stderr,"compiler_version_harness_failure":version.harness_failure,"compiler_sha256":compiler_hash});};
        let solver_version=command_at(&solver,&["-version".into()]);let solver_hash=hash_file(&solver);
        json!({"valid":version.success && version.harness_failure.is_none() && solver_version.success && solver_version.harness_failure.is_none() && compiler_hash.is_some() && solver_hash.is_some(),"compiler_sha256":compiler_hash,"compiler_version_harness_failure":version.harness_failure,"solver_version_harness_failure":solver_version.harness_failure,"compiler_version":version.stdout,"compiler_version_code":version.code,"compiler_version_stderr":version.stderr,"solver_path":solver,"solver_sha256":solver_hash,"solver_version":solver_version.stdout,"solver_version_code":solver_version.code,"solver_version_stderr":solver_version.stderr,"environment_limit":"dynamic loader/shared-library/Python closure NOT collected or claimed by this diagnostic"})
    })
}

struct Fixture {
    root: PathBuf,
    case: &'static str,
    sources: BTreeMap<String, String>,
}
impl Fixture {
    fn new(case: &'static str, main: &str, dependency: Option<&str>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "export_collision_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let mut sources = BTreeMap::from([("program.rz".to_string(), main.to_string())]);
        if let Some(dep) = dependency {
            sources.insert("dep.rz".into(), dep.into());
        }
        for (path, text) in &sources {
            std::fs::write(root.join(path), text).unwrap();
        }
        let key = SigningKey::generate(&mut rand_core::OsRng);
        for (name, label, bytes) in [
            ("private.pem", "PRIVATE", key.to_bytes()),
            ("public.pem", "PUBLIC", key.verifying_key().to_bytes()),
        ] {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            std::fs::write(root.join(name),format!("-----BEGIN ED25519 {label} KEY-----\n{hex}\n-----END ED25519 {label} KEY-----\n")).unwrap();
        }
        Self {
            root,
            case,
            sources,
        }
    }
    fn run(&self, stage: &str, argv: Vec<String>) -> Capture {
        let output = command_at(Path::new(env!("CARGO_BIN_EXE_rz")), &argv);
        let snapshot = artifact_snapshot(&self.root.join("certs"));
        let (files, snapshot_failure) = match snapshot {
            Ok(files) => (files, None),
            Err(e) => (BTreeMap::new(), Some(e)),
        };
        let (public_key, public_key_sha256, public_key_failure) =
            match bounded_regular(&self.root.join("public.pem"), 4096) {
                Ok(bytes) => match String::from_utf8(bytes.clone()) {
                    Ok(text) => (Some(text), Some(hash(&bytes)), None),
                    Err(_) => (None, None, Some("public key is not UTF8".to_string())),
                },
                Err(error) => (None, None, Some(error)),
            };
        let record = json!({"schema":"resilient-export-collision-diagnostic/v2","case":self.case,"stage":stage,
            "argv":argv,"code":output.code,"success":output.success,"timed_out":output.timed_out,"harness_failure":output.harness_failure,"artifact_snapshot_failure":snapshot_failure,
            "stdout":output.stdout,"stderr":output.stderr,"source_units":self.sources.iter().map(|(path,source)|json!({"path":path,"source":source,"sha256":hash(source.as_bytes())})).collect::<Vec<_>>(),
            "compiler_path":env!("CARGO_BIN_EXE_rz"),"compiler_sha256":identities()["compiler_sha256"],
            "public_key":public_key,"public_key_sha256":public_key_sha256,"public_key_failure":public_key_failure,
            "identities":identities(),"certificate_files":files,"profile":{"feature_z3":true,"feature_jit":cfg!(feature="jit"),"overflow_environment":"unset"},
            "claim":"public export/replay integrity diagnostic; not source-to-SMT soundness; private key intentionally excluded"});
        let mut out = std::io::stdout().lock();
        writeln!(out, "EXPORT_COLLISION_RECORD {record}").unwrap();
        out.flush().unwrap();
        assert!(
            output.harness_failure.is_none()
                && snapshot_failure.is_none()
                && public_key_failure.is_none(),
            "{stage}: harness/artifact failure recorded; not collision evidence"
        );
        assert_eq!(
            identities()["valid"],
            true,
            "invalid identity stage recorded; no collision evidence"
        );
        output
    }
    fn preflight(&self) {
        let p = self.run(
            "frontend_preflight",
            vec![
                "--no-cache".into(),
                "-t".into(),
                self.root.join("program.rz").display().to_string(),
            ],
        );
        assert!(
            p.success && p.stdout.contains("Type check passed"),
            "FRONTEND_UNREACHED_NOT_COLLISION_EVIDENCE: {} {}",
            p.stdout,
            p.stderr
        );
        // -t enables checking but may continue execution; fixtures contain declarations only.
    }
    fn export(&self, allow_explicit_fail_closed: bool) -> Option<Value> {
        self.preflight();
        let p = self.run(
            "signed_export",
            vec![
                "--no-cache".into(),
                "-t".into(),
                "--emit-certificate".into(),
                self.root.join("certs").display().to_string(),
                "--sign-cert".into(),
                self.root.join("private.pem").display().to_string(),
                self.root.join("program.rz").display().to_string(),
            ],
        );
        if !p.success {
            let diagnostic = p.stderr.to_ascii_lowercase();
            let export_specific = diagnostic.contains("certificate")
                && (diagnostic.contains("collision")
                    || diagnostic.contains("duplicate artifact")
                    || diagnostic.contains("export unsupported"));
            assert!(
                allow_explicit_fail_closed
                    && export_specific
                    && !diagnostic.contains("parse error")
                    && !diagnostic.contains("type check failed"),
                "EXPORT_UNREACHED_OR_UNCLASSIFIED_NOT_COLLISION_EVIDENCE: {} {}",
                p.stdout,
                p.stderr
            );
            return None;
        }
        assert!(
            p.stdout.contains("Type check passed"),
            "successful export without observed frontend stage"
        );
        Some(
            serde_json::from_slice(
                &bounded_regular(&self.root.join("certs/manifest.json"), 262_144).unwrap(),
            )
            .unwrap(),
        )
    }
    fn obligations(&self, manifest: &Value, kind: &str, label: Option<&str>) -> Vec<Value> {
        manifest["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["kind"] == kind && label.is_none_or(|n| o["fn"] == n))
            .cloned()
            .collect()
    }
    fn consistent_and_replay(&self, manifest: &Value) {
        let p = self.run(
            "strict_public_replay",
            vec![
                "verify-all".into(),
                self.root.join("certs").display().to_string(),
                "--pubkey".into(),
                self.root.join("public.pem").display().to_string(),
                "--require-manifest-signature".into(),
                "--source-root".into(),
                self.root.display().to_string(),
                "--z3".into(),
            ],
        );
        // Sharing one blob is not prohibited if every explicit obligation binds those SAME bytes.
        // Different query hashes for one path can never satisfy this requirement.
        for o in manifest["obligations"].as_array().unwrap() {
            let name = o["cert"].as_str().unwrap();
            let relative = Path::new(name);
            assert!(
                relative
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
                "unsafe artifact path"
            );
            assert_eq!(
                o["sha256"],
                hash(&bounded_regular(&self.root.join("certs").join(relative), 262_144).unwrap()),
                "SILENT_OVERWRITE_OR_INCONSISTENT_OBLIGATION {o}"
            );
        }
        assert!(
            p.success,
            "SUCCESSFUL_EXPORT_PRODUCED_INVALID_BUNDLE: {} {}",
            p.stdout, p.stderr
        );
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const IDENTITY: &str = "fn f(int x) -> int ensures result == x { return x; }";
const CONSTANT: &str = "fn left__f() -> int ensures result == 7 { return 7; }";
const CALLEE: &str =
    "fn guarded(int x, int offset) -> int requires x * x + offset >= offset { return x; }";
fn only_query(f: &Fixture, kind: &str, label: &str) -> String {
    let m = f.export(false).unwrap();
    let o = f.obligations(&m, kind, Some(label));
    assert_eq!(
        o.len(),
        1,
        "CONTROL_QUERY_CAPTURE_INELIGIBLE_NOT_COLLISION_EVIDENCE {m}"
    );
    f.consistent_and_replay(&m);
    o[0]["sha256"].as_str().unwrap().to_string()
}
#[test]
fn independent_plain_function_exports_are_replayable() {
    let f = Fixture::new(
        "plain_control",
        "fn left_f(int x) -> int ensures result == x { return x; } fn left__f() -> int ensures result == 7 { return 7; }",
        None,
    );
    let m = f.export(false).unwrap();
    assert_eq!(
        f.obligations(&m, "ensures_implementation", None).len(),
        2,
        "control must actually capture two implementation queries"
    );
    f.consistent_and_replay(&m);
}
#[test]
fn aliased_file_and_plain_name_preserve_distinct_queries() {
    let a = Fixture::new(
        "alias_isolated_control",
        "use \"dep.rz\" as left;",
        Some(IDENTITY),
    );
    let ah = only_query(&a, "ensures_implementation", "left::f:ensures:0");
    let b = Fixture::new("plain_isolated_control", CONSTANT, None);
    let bh = only_query(&b, "ensures_implementation", "left__f:ensures:0");
    assert_ne!(ah, bh, "INDISTINCT_CONTROL_QUERIES_NOT_OVERWRITE_EVIDENCE");
    let f = Fixture::new(
        "alias_collision",
        "use \"dep.rz\" as left; fn left__f() -> int ensures result == 7 { return 7; }",
        Some(IDENTITY),
    );
    if let Some(m) = f.export(true) {
        f.consistent_and_replay(&m);
        let a = f.obligations(&m, "ensures_implementation", Some("left::f:ensures:0"));
        let b = f.obligations(&m, "ensures_implementation", Some("left__f:ensures:0"));
        assert_eq!(a.len(), 1, "missing original alias obligation {m}");
        assert_eq!(b.len(), 1, "missing original plain obligation {m}");
        assert_eq!(
            a[0]["sha256"], ah,
            "ORACLE_QUERYBINDING_UNRESOLVED alias query changed in combined context; NOT compiler defect without independent structure evidence"
        );
        assert_eq!(
            b[0]["sha256"], bh,
            "ORACLE_QUERYBINDING_UNRESOLVED plain query changed in combined context; NOT compiler defect without independent structure evidence"
        );
    }
}
#[test]
fn repeated_callsite_captures_preserve_distinct_bound_queries() {
    let one = format!("{CALLEE} fn one(int a) -> int {{ return guarded(a, 1); }}");
    let two = format!("{CALLEE} fn two(int b) -> int {{ return guarded(b, 2); }}");
    let a = Fixture::new("call_one_control", &one, None);
    let ah = only_query(&a, "callsite_requires", "guarded");
    let b = Fixture::new("call_two_control", &two, None);
    let bh = only_query(&b, "callsite_requires", "guarded");
    assert_ne!(ah, bh, "INDISTINCT_CALL_CONTROLS_NOT_OVERWRITE_EVIDENCE");
    let source = format!(
        "{CALLEE} fn one(int a) -> int {{ return guarded(a, 1); }} fn two(int b) -> int {{ return guarded(b, 2); }}"
    );
    let f = Fixture::new("call_collision", &source, None);
    if let Some(m) = f.export(true) {
        f.consistent_and_replay(&m);
        let o = f.obligations(&m, "callsite_requires", Some("guarded"));
        assert_eq!(o.len(), 2, "missing original callsite obligation {m}");
        let mut actual: Vec<_> = o
            .iter()
            .map(|x| x["sha256"].as_str().unwrap().to_string())
            .collect();
        let mut expected = vec![ah, bh];
        actual.sort();
        expected.sort();
        assert_eq!(
            actual, expected,
            "ORACLE_QUERYBINDING_UNRESOLVED call query changed in combined context; NOT compiler defect without independent structure evidence"
        );
    }
}
