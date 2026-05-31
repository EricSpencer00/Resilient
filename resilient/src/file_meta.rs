//! RES-2557: File metadata builtins — file_exists, file_stat, file_is_dir,
//! file_is_file, file_size, dir_list (all std-only).

use crate::Value;
use std::path::Path;

type RResult<T> = Result<T, String>;

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

/// `file_exists(path: string) -> bool`
///
/// Returns true if anything (file, directory, symlink) exists at `path`.
pub(crate) fn builtin_file_exists(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).exists())),
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
/// Returns true if `path` exists and is a directory.
pub(crate) fn builtin_file_is_dir(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).is_dir())),
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
/// Returns true if `path` exists and is a regular file.
pub(crate) fn builtin_file_is_file(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path)] => Ok(Value::Bool(Path::new(path.as_str()).is_file())),
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
/// Returns `Ok(size_in_bytes)` or `Err(message)`.
pub(crate) fn builtin_file_size(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(path)] => match std::fs::metadata(path.as_str()) {
            Ok(meta) => Ok(ok(Value::Int(meta.len() as i64))),
            Err(e) => Ok(err(format!("file_size: {}: {}", path, e))),
        },
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
pub(crate) fn builtin_file_stat(args: &[Value]) -> RResult<Value> {
    match args {
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
pub(crate) fn builtin_dir_list(args: &[Value]) -> RResult<Value> {
    match args {
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
        [other] => Err(format!("dir_list: expected string path, got {:?}", other)),
        _ => Err(format!("dir_list: expected 1 argument, got {}", args.len())),
    }
}
