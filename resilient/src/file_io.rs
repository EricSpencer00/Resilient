//! RES-409: streaming file I/O — `file_open`, `file_read_chunk`,
//! `file_seek`, `file_close`. Memory-bounded reads (the only safe
//! kind on embedded with constrained RAM) finally possible.
//!
//! A file handle is encoded as a `Value::Struct { name: "File",
//! fields: [("id", Int(N))] }` so the Value enum doesn't need a new
//! variant — the runtime registry below is keyed by `N`. The struct
//! shape doubles as the user-visible value (`f.id` is the handle id
//! if anyone needs it for debugging).
//!
//! Linear-type integration: a future RES-385 follow-up can mark
//! `File` as linear by adding it to the linear-type module's
//! resource list. For the MVP, single-use is enforced by the runtime
//! itself: `file_close` drops the handle from the registry, and any
//! subsequent operation on it surfaces a `closed-or-unknown handle`
//! diagnostic.
//!
//! Std-only — every builtin here pulls in `std::fs`. The
//! `resilient-runtime` sibling crate has no builtins table at all and
//! stays no_std-clean.
//!
//! RES-3877: on `wasm32` (the web playground) there is no host
//! filesystem — `std::fs` calls return `Unsupported` at runtime, so
//! the file-I/O examples fail. There, `Backend` is an in-memory VFS
//! (`wasm_vfs::WasmFile`, a `Cursor<Vec<u8>>` over a thread-local
//! path→bytes store) so the same builtins demonstrate the language
//! semantics without a real filesystem. The read / write / seek /
//! close handlers below are backend-agnostic: they only touch the
//! `Read + Write + Seek` surface, so a single implementation drives
//! both. Only `file_open` — which constructs the backing handle —
//! forks per target.

use crate::{RResult, Value};
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicI64, Ordering};

/// Handle backend. Native builds hold a real OS file; the wasm
/// playground holds an in-memory VFS handle (RES-3877).
#[cfg(not(target_arch = "wasm32"))]
type Backend = File;
#[cfg(target_arch = "wasm32")]
type Backend = wasm_vfs::WasmFile;

/// Process-global handle id counter. Each `file_open` mints the next
/// id; ids never recycle within a process so a stale handle can't
/// silently alias a freshly opened file.
static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

thread_local! {
    static REGISTRY: RefCell<HashMap<i64, Backend>> = RefCell::new(HashMap::new());
}

/// `file_open(path: String, mode: String) -> Result<File, String>`.
/// Modes: `"r"` (read-only), `"w"` (write-only, truncate), `"rw"`
/// (read+write, create if missing, do not truncate).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn builtin_file_open(args: &[Value]) -> RResult<Value> {
    let (path, mode) = match args {
        [Value::String(p), Value::String(m)] => (p, m),
        _ => {
            return Err(format!(
                "file_open: expected (String path, String mode), got {} arg(s)",
                args.len()
            ));
        }
    };
    let result = match mode.as_str() {
        "r" => OpenOptions::new().read(true).open(path),
        "w" => OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path),
        "rw" => OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path),
        other => {
            return Ok(Value::Result {
                ok: false,
                payload: Box::new(Value::String(format!(
                    "file_open: unknown mode `{}` (expected r / w / rw)",
                    other
                ))),
            });
        }
    };
    match result {
        Ok(file) => {
            let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
            REGISTRY.with(|r| {
                r.borrow_mut().insert(id, file);
            });
            Ok(Value::Result {
                ok: true,
                payload: Box::new(handle_value(id)),
            })
        }
        Err(e) => Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!("file_open: {}: {}", path, e))),
        }),
    }
}

/// RES-3877: wasm playground `file_open` — backed by the in-memory
/// VFS. Same mode contract as the native handler; misses on `"r"`
/// surface as `Err`, mirroring "no such file" on a real filesystem.
#[cfg(target_arch = "wasm32")]
pub(crate) fn builtin_file_open(args: &[Value]) -> RResult<Value> {
    let (path, mode) = match args {
        [Value::String(p), Value::String(m)] => (p, m),
        _ => {
            return Err(format!(
                "file_open: expected (String path, String mode), got {} arg(s)",
                args.len()
            ));
        }
    };
    match mode.as_str() {
        "r" | "w" | "rw" => match wasm_vfs::open(path, mode) {
            Ok(file) => {
                let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
                REGISTRY.with(|r| {
                    r.borrow_mut().insert(id, file);
                });
                Ok(Value::Result {
                    ok: true,
                    payload: Box::new(handle_value(id)),
                })
            }
            Err(e) => Ok(Value::Result {
                ok: false,
                payload: Box::new(Value::String(format!("file_open: {}: {}", path, e))),
            }),
        },
        other => Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!(
                "file_open: unknown mode `{}` (expected r / w / rw)",
                other
            ))),
        }),
    }
}

/// `file_read_chunk(handle: File, max_bytes: Int) -> Result<Bytes, String>`.
/// Reads up to `max_bytes` bytes from the current cursor position.
/// Empty `Bytes` indicates EOF.
pub(crate) fn builtin_file_read_chunk(args: &[Value]) -> RResult<Value> {
    let (id, max) = match args {
        [Value::Struct { name, fields }, Value::Int(n)] if name == "File" => {
            let id = handle_id_from_fields(fields)?;
            (id, *n)
        }
        _ => {
            return Err(format!(
                "file_read_chunk: expected (File, Int), got {} arg(s)",
                args.len()
            ));
        }
    };
    if max < 0 {
        return Err(format!(
            "file_read_chunk: max_bytes must be non-negative, got {}",
            max
        ));
    }
    let max_usize = max as usize;
    let result = REGISTRY.with(|r| -> Result<Vec<u8>, std::io::Error> {
        let mut reg = r.borrow_mut();
        let f = reg
            .get_mut(&id)
            .ok_or_else(|| std::io::Error::other("closed or unknown file handle"))?;
        let mut buf = vec![0u8; max_usize];
        let n = f.read(&mut buf)?;
        buf.truncate(n);
        Ok(buf)
    });
    match result {
        Ok(bytes) => Ok(Value::Result {
            ok: true,
            payload: Box::new(Value::Bytes(bytes)),
        }),
        Err(e) => Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!("file_read_chunk: {}", e))),
        }),
    }
}

/// `file_write_chunk(handle: File, bytes: Bytes) -> Result<Int, String>`.
/// Writes the entire byte slice; returns the number of bytes written.
pub(crate) fn builtin_file_write_chunk(args: &[Value]) -> RResult<Value> {
    let (id, bytes) = match args {
        [Value::Struct { name, fields }, Value::Bytes(b)] if name == "File" => {
            let id = handle_id_from_fields(fields)?;
            (id, b.clone())
        }
        _ => {
            return Err(format!(
                "file_write_chunk: expected (File, Bytes), got {} arg(s)",
                args.len()
            ));
        }
    };
    let result = REGISTRY.with(|r| -> Result<usize, std::io::Error> {
        let mut reg = r.borrow_mut();
        let f = reg
            .get_mut(&id)
            .ok_or_else(|| std::io::Error::other("closed or unknown file handle"))?;
        f.write_all(&bytes)?;
        Ok(bytes.len())
    });
    match result {
        Ok(n) => Ok(Value::Result {
            ok: true,
            payload: Box::new(Value::Int(n as i64)),
        }),
        Err(e) => Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!("file_write_chunk: {}", e))),
        }),
    }
}

/// `file_seek(handle: File, offset: Int, whence: String) -> Result<Int, String>`.
/// Whence is one of `"start"`, `"current"`, `"end"`. Returns the new
/// cursor position from the start of the file.
pub(crate) fn builtin_file_seek(args: &[Value]) -> RResult<Value> {
    let (id, offset, whence) = match args {
        [
            Value::Struct { name, fields },
            Value::Int(o),
            Value::String(w),
        ] if name == "File" => {
            let id = handle_id_from_fields(fields)?;
            (id, *o, w.clone())
        }
        _ => {
            return Err(format!(
                "file_seek: expected (File, Int, String), got {} arg(s)",
                args.len()
            ));
        }
    };
    let seek_from = match whence.as_str() {
        "start" => {
            if offset < 0 {
                return Err(format!(
                    "file_seek: `start` whence requires non-negative offset, got {}",
                    offset
                ));
            }
            SeekFrom::Start(offset as u64)
        }
        "current" => SeekFrom::Current(offset),
        "end" => SeekFrom::End(offset),
        other => {
            return Ok(Value::Result {
                ok: false,
                payload: Box::new(Value::String(format!(
                    "file_seek: unknown whence `{}` (expected start / current / end)",
                    other
                ))),
            });
        }
    };
    let result = REGISTRY.with(|r| -> Result<u64, std::io::Error> {
        let mut reg = r.borrow_mut();
        let f = reg
            .get_mut(&id)
            .ok_or_else(|| std::io::Error::other("closed or unknown file handle"))?;
        f.seek(seek_from)
    });
    match result {
        Ok(pos) => Ok(Value::Result {
            ok: true,
            payload: Box::new(Value::Int(pos as i64)),
        }),
        Err(e) => Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(format!("file_seek: {}", e))),
        }),
    }
}

/// `file_close(handle: File) -> Result<Void, String>`. Removes the
/// handle from the registry; the underlying `std::fs::File` is dropped
/// (and flushed) at that point. A second close returns `Err("already
/// closed")` so the linear-use violation is loud, not silent.
pub(crate) fn builtin_file_close(args: &[Value]) -> RResult<Value> {
    let id = match args {
        [Value::Struct { name, fields }] if name == "File" => handle_id_from_fields(fields)?,
        _ => {
            return Err(format!(
                "file_close: expected (File,), got {} arg(s)",
                args.len()
            ));
        }
    };
    let removed = REGISTRY.with(|r| r.borrow_mut().remove(&id).is_some());
    if removed {
        Ok(Value::Result {
            ok: true,
            payload: Box::new(Value::Void),
        })
    } else {
        Ok(Value::Result {
            ok: false,
            payload: Box::new(Value::String(
                "file_close: already closed or unknown handle".to_string(),
            )),
        })
    }
}

fn handle_value(id: i64) -> Value {
    Value::Struct {
        name: "File".to_string(),
        fields: vec![("id".to_string(), Value::Int(id))],
    }
}

fn handle_id_from_fields(fields: &[(String, Value)]) -> Result<i64, String> {
    for (k, v) in fields {
        if k == "id"
            && let Value::Int(n) = v
        {
            return Ok(*n);
        }
    }
    Err("file handle struct is missing `id: Int` field".to_string())
}

/// RES-3877: in-memory virtual filesystem backing the file-I/O
/// builtins on `wasm32` (and exercised natively under `cfg(test)`).
/// A thread-local path→bytes store stands in for the host filesystem;
/// `WasmFile` is a seekable cursor over a handle's working copy that
/// mirrors back to the store on every write.
#[cfg(any(target_arch = "wasm32", test))]
mod wasm_vfs {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

    thread_local! {
        static STORE: RefCell<HashMap<String, Vec<u8>>> = RefCell::new(HashMap::new());
    }

    pub(super) fn store_write(path: &str, bytes: &[u8]) {
        STORE.with(|s| {
            s.borrow_mut().insert(path.to_string(), bytes.to_vec());
        });
    }

    pub(super) fn store_read(path: &str) -> io::Result<Vec<u8>> {
        STORE.with(|s| {
            s.borrow()
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no such file"))
        })
    }

    /// Seekable in-memory file handle. Writes overwrite at the cursor
    /// and immediately mirror the full buffer back to the store so a
    /// later `file_read` / re-open observes them.
    pub(super) struct WasmFile {
        path: String,
        cursor: Cursor<Vec<u8>>,
        writable: bool,
    }

    /// Open `path` in `mode` against the store. `"r"` requires the
    /// file to exist; `"w"` truncates (creating an empty file); `"rw"`
    /// opens the existing bytes (or empty) without truncating.
    pub(super) fn open(path: &str, mode: &str) -> io::Result<WasmFile> {
        let (initial, writable) = match mode {
            "r" => (store_read(path)?, false),
            "w" => {
                store_write(path, &[]);
                (Vec::new(), true)
            }
            "rw" => (store_read(path).unwrap_or_default(), true),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown mode `{}`", other),
                ));
            }
        };
        Ok(WasmFile {
            path: path.to_string(),
            cursor: Cursor::new(initial),
            writable,
        })
    }

    impl Read for WasmFile {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.cursor.read(buf)
        }
    }

    impl Seek for WasmFile {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.cursor.seek(pos)
        }
    }

    impl Write for WasmFile {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if !self.writable {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "file not opened for writing",
                ));
            }
            let n = self.cursor.write(buf)?;
            store_write(&self.path, self.cursor.get_ref());
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            store_write(&self.path, self.cursor.get_ref());
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn write_then_read_round_trips() {
            store_write("/vfs/a.txt", b"hello");
            assert_eq!(store_read("/vfs/a.txt").unwrap(), b"hello");
        }

        #[test]
        fn read_missing_is_not_found() {
            let err = store_read("/vfs/does-not-exist").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        }

        #[test]
        fn open_r_requires_existing_file() {
            assert!(open("/vfs/missing-r", "r").is_err());
        }

        #[test]
        fn open_w_truncates_and_writes_flush_to_store() {
            store_write("/vfs/w.txt", b"stale contents");
            let mut f = open("/vfs/w.txt", "w").unwrap();
            assert_eq!(store_read("/vfs/w.txt").unwrap(), b"");
            f.write_all(b"fresh").unwrap();
            assert_eq!(store_read("/vfs/w.txt").unwrap(), b"fresh");
        }

        #[test]
        fn open_r_rejects_writes() {
            store_write("/vfs/ro.txt", b"abc");
            let mut f = open("/vfs/ro.txt", "r").unwrap();
            let err = f.write(b"x").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn seek_then_read_repositions_cursor() {
            store_write("/vfs/seek.txt", b"abcdefgh");
            let mut f = open("/vfs/seek.txt", "r").unwrap();
            f.seek(SeekFrom::Start(3)).unwrap();
            let mut buf = [0u8; 3];
            f.read_exact(&mut buf).unwrap();
            assert_eq!(&buf, b"def");
        }

        #[test]
        fn rw_preserves_existing_bytes() {
            store_write("/vfs/rw.txt", b"keep");
            let mut f = open("/vfs/rw.txt", "rw").unwrap();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            assert_eq!(buf, b"keep");
        }
    }
}

/// RES-3877: playground-facing VFS accessors used by the simple
/// `file_read` / `file_write` builtins in `lib.rs`. They share the
/// same store as the streaming handlers so `file_write` followed by
/// `file_open` / `file_read` round-trips inside the sandbox.
#[cfg(target_arch = "wasm32")]
pub(crate) fn vfs_write(path: &str, contents: &[u8]) {
    wasm_vfs::store_write(path, contents);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn vfs_read(path: &str) -> std::io::Result<Vec<u8>> {
    wasm_vfs::store_read(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("resilient-res409-{}-{}", name, std::process::id()));
        p
    }

    fn open_handle_id(v: &Value) -> i64 {
        match v {
            Value::Result { ok: true, payload } => match payload.as_ref() {
                Value::Struct { name, fields } if name == "File" => {
                    handle_id_from_fields(fields).unwrap()
                }
                other => panic!("expected File struct, got {:?}", other),
            },
            other => panic!("expected Ok payload, got {:?}", other),
        }
    }

    #[test]
    fn open_read_close_round_trip() {
        let path = temp_path("rw");
        std::fs::write(&path, b"hello world").unwrap();

        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("r".into()),
        ])
        .unwrap();
        let id = open_handle_id(&opened);
        let handle = handle_value(id);

        let chunk = builtin_file_read_chunk(&[handle.clone(), Value::Int(5)]).unwrap();
        let Value::Result { ok: true, payload } = chunk else {
            panic!("expected Ok, got {:?}", chunk);
        };
        let Value::Bytes(b) = *payload else {
            panic!("expected Bytes payload");
        };
        assert_eq!(b, b"hello");

        let close = builtin_file_close(&[handle]).unwrap();
        assert!(matches!(close, Value::Result { ok: true, .. }));

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_chunk_returns_empty_at_eof() {
        let path = temp_path("eof");
        std::fs::write(&path, b"abc").unwrap();
        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("r".into()),
        ])
        .unwrap();
        let handle = handle_value(open_handle_id(&opened));

        // Drain the file.
        let _ = builtin_file_read_chunk(&[handle.clone(), Value::Int(1024)]).unwrap();
        // Subsequent read returns Ok(Bytes(empty)).
        let again = builtin_file_read_chunk(&[handle.clone(), Value::Int(1024)]).unwrap();
        let Value::Result { ok: true, payload } = again else {
            panic!("expected Ok, got {:?}", again);
        };
        let Value::Bytes(b) = *payload else {
            panic!("expected Bytes payload");
        };
        assert!(b.is_empty(), "expected empty bytes at EOF, got {:?}", b);

        builtin_file_close(&[handle]).unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn double_close_errors() {
        let path = temp_path("double-close");
        std::fs::write(&path, b"x").unwrap();
        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("r".into()),
        ])
        .unwrap();
        let handle = handle_value(open_handle_id(&opened));

        let first = builtin_file_close(std::slice::from_ref(&handle)).unwrap();
        assert!(matches!(first, Value::Result { ok: true, .. }));

        let second = builtin_file_close(&[handle]).unwrap();
        let Value::Result { ok: false, payload } = second else {
            panic!("expected Err, got {:?}", second);
        };
        let Value::String(msg) = *payload else {
            panic!("expected String error payload");
        };
        assert!(msg.contains("already closed"), "unexpected error: {}", msg);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn seek_repositions_cursor() {
        let path = temp_path("seek");
        std::fs::write(&path, b"abcdefgh").unwrap();
        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("r".into()),
        ])
        .unwrap();
        let handle = handle_value(open_handle_id(&opened));

        // Seek to byte 3, read 3 bytes → "def".
        let seek =
            builtin_file_seek(&[handle.clone(), Value::Int(3), Value::String("start".into())])
                .unwrap();
        assert!(matches!(seek, Value::Result { ok: true, .. }));

        let chunk = builtin_file_read_chunk(&[handle.clone(), Value::Int(3)]).unwrap();
        let Value::Result { ok: true, payload } = chunk else {
            panic!("expected Ok");
        };
        let Value::Bytes(b) = *payload else {
            panic!("expected Bytes")
        };
        assert_eq!(b, b"def");

        builtin_file_close(&[handle]).unwrap();
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn open_unknown_mode_errors() {
        let opened = builtin_file_open(&[
            Value::String("/dev/null".into()),
            Value::String("zzz".into()),
        ])
        .unwrap();
        let Value::Result { ok: false, payload } = opened else {
            panic!("expected Err on bad mode");
        };
        let Value::String(msg) = *payload else {
            panic!("expected String error");
        };
        assert!(msg.contains("unknown mode"), "unexpected: {}", msg);
    }

    #[test]
    fn open_missing_file_errors() {
        let opened = builtin_file_open(&[
            Value::String("/nonexistent-resilient-res409-xxx".into()),
            Value::String("r".into()),
        ])
        .unwrap();
        assert!(
            matches!(opened, Value::Result { ok: false, .. }),
            "expected Err for missing file"
        );
    }

    #[test]
    fn read_after_close_errors() {
        let path = temp_path("read-after-close");
        std::fs::write(&path, b"x").unwrap();
        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("r".into()),
        ])
        .unwrap();
        let handle = handle_value(open_handle_id(&opened));

        builtin_file_close(std::slice::from_ref(&handle)).unwrap();
        let read = builtin_file_read_chunk(&[handle, Value::Int(1)]).unwrap();
        let Value::Result { ok: false, payload } = read else {
            panic!("expected Err on read-after-close");
        };
        let Value::String(msg) = *payload else {
            panic!("expected String payload");
        };
        assert!(msg.contains("closed or unknown"), "unexpected: {}", msg);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn write_chunk_round_trip() {
        let path = temp_path("write");
        let _ = std::fs::remove_file(&path);

        let opened = builtin_file_open(&[
            Value::String(path.to_string_lossy().into()),
            Value::String("w".into()),
        ])
        .unwrap();
        let handle = handle_value(open_handle_id(&opened));

        let written =
            builtin_file_write_chunk(&[handle.clone(), Value::Bytes(b"hello".to_vec())]).unwrap();
        let Value::Result { ok: true, payload } = written else {
            panic!("expected Ok");
        };
        assert!(matches!(*payload, Value::Int(5)));

        builtin_file_close(&[handle]).unwrap();

        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk, b"hello");

        std::fs::remove_file(&path).ok();
    }
}
