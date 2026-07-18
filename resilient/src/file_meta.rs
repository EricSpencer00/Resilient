//! RES-2557: File metadata builtins — file_exists, file_stat, file_is_dir,
//! file_is_file, file_size, dir_list (all std-only).
//!
//! RES-4126: on `wasm32` (the web playground) there is no host
//! filesystem to stat — these builtins never see real files (the
//! playground's `file_read`/`file_write` route through an in-memory
//! VFS instead, see `file_io.rs`). Rather than let `std::fs::metadata`
//! / `std::fs::read_dir` behave unpredictably against whatever the
//! wasm host happens to expose, every builtin here short-circuits on
//! `wasm32` with a clear "unsupported on this target" `Err`, mirroring
//! the graceful-Err pattern used for `http_client.rs` / `process_exec.rs`.

use crate::Value;
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

type RResult<T> = Result<T, String>;

#[cfg(not(target_arch = "wasm32"))]
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
fn unsupported(builtin: &str) -> String {
    format!(
        "{}: unsupported on this target (no host filesystem in the wasm playground)",
        builtin
    )
}

/// `file_exists(path: string) -> bool`
///
/// Returns true if anything (file, directory, symlink) exists at `path`.
/// On `wasm32` there is no host filesystem, so this always returns
/// `false` rather than panicking or hanging on an unsupported syscall.
pub(crate) fn builtin_file_exists(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).exists())),
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(Value::Bool(false)),
        [other] => Err(format!(
            "file_exists: expected string path, got {:?}",
            other
        )),
        _ => Err(format!(
            "file_exists: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `file_is_dir(path: string) -> bool`
///
/// Returns true if `path` exists and is a directory. Always `false`
/// on `wasm32` (see `file_exists`).
pub(crate) fn builtin_file_is_dir(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).is_dir())),
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(Value::Bool(false)),
        [other] => Err(format!(
            "file_is_dir: expected string path, got {:?}",
            other
        )),
        _ => Err(format!(
            "file_is_dir: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `file_is_file(path: string) -> bool`
///
/// Returns true if `path` exists and is a regular file. Always `false`
/// on `wasm32` (see `file_exists`).
pub(crate) fn builtin_file_is_file(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).is_file())),
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(Value::Bool(false)),
        [other] => Err(format!(
            "file_is_file: expected string path, got {:?}",
            other
        )),
        _ => Err(format!(
            "file_is_file: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `file_size(path: string) -> Result<int, string>`
///
/// Returns `Ok(size_in_bytes)` or `Err(message)`. On `wasm32` always
/// returns `Err` — there is no host filesystem to stat.
pub(crate) fn builtin_file_size(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => match std::fs::metadata(path.as_str()) {
            Ok(meta) => Ok(ok(Value::Int(meta.len() as i64))),
            Err(e) => Ok(err(format!("file_size: {}: {}", path, e))),
        },
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(err(unsupported("file_size"))),
        [other] => Err(format!("file_size: expected string path, got {:?}", other)),
        _ => Err(format!(
            "file_size: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `file_stat(path: string) -> Result<FileMeta, string>`
///
/// Returns a `FileMeta` struct with fields:
///   - `size: int`       — byte count
///   - `modified: int`   — seconds since Unix epoch (0 if unavailable)
///   - `is_dir: bool`    — is directory
///   - `is_file: bool`   — is regular file
///
/// On `wasm32` always returns `Err` — there is no host filesystem to stat.
pub(crate) fn builtin_file_stat(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => match std::fs::metadata(path.as_str()) {
            Ok(meta) => {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let file_meta = Value::Struct {
                    name: "FileMeta".to_string(),
                    fields: vec![
                        ("size".to_string(), Value::Int(meta.len() as i64)),
                        ("modified".to_string(), Value::Int(modified)),
                        ("is_dir".to_string(), Value::Bool(meta.is_dir())),
                        ("is_file".to_string(), Value::Bool(meta.is_file())),
                    ],
                };
                Ok(ok(file_meta))
            }
            Err(e) => Ok(err(format!("file_stat: {}: {}", path, e))),
        },
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(err(unsupported("file_stat"))),
        [other] => Err(format!("file_stat: expected string path, got {:?}", other)),
        _ => Err(format!(
            "file_stat: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `dir_list(path: string) -> Result<[string], string>`
///
/// Returns a sorted list of entry names (not full paths) inside `path`.
/// On `wasm32` always returns `Err` — there is no host filesystem to list.
pub(crate) fn builtin_dir_list(args: &[Value]) -> RResult<Value> {
    match args {
        #[cfg(not(target_arch = "wasm32"))]
        [Value::String(path)] => match std::fs::read_dir(path.as_str()) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .filter_map(|e| {
                        e.ok()
                            .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    })
                    .collect();
                names.sort();
                let arr = Value::Array(names.into_iter().map(Value::String).collect());
                Ok(ok(arr))
            }
            Err(e) => Ok(err(format!("dir_list: {}: {}", path, e))),
        },
        #[cfg(target_arch = "wasm32")]
        [Value::String(_path)] => Ok(err(unsupported("dir_list"))),
        [other] => Err(format!("dir_list: expected string path, got {:?}", other)),
        _ => Err(format!("dir_list: expected 1 argument, got {}", args.len())),
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use super::*;

    #[test]
    fn file_exists_is_false_on_wasm32() {
        let r = builtin_file_exists(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Bool(false)));
    }

    #[test]
    fn file_is_dir_is_false_on_wasm32() {
        let r = builtin_file_is_dir(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Bool(false)));
    }

    #[test]
    fn file_is_file_is_false_on_wasm32() {
        let r = builtin_file_is_file(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Bool(false)));
    }

    #[test]
    fn file_size_errs_on_wasm32() {
        let r = builtin_file_size(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Result { ok: false, .. }));
    }

    #[test]
    fn file_stat_errs_on_wasm32() {
        let r = builtin_file_stat(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Result { ok: false, .. }));
    }

    #[test]
    fn dir_list_errs_on_wasm32() {
        let r = builtin_dir_list(&[Value::String("/anything".to_string())]).unwrap();
        assert!(matches!(r, Value::Result { ok: false, .. }));
    }
}
