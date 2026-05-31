//! RES-2583: Mutex and RwLock synchronization primitives.
//!
//! Provides interpreter-level wrappers around shared-state concurrency
//! primitives. In the single-threaded interpreter these are value containers
//! with explicit lock/unlock; compiled backends map them to real OS primitives.
//!
//! ## API
//!
//!   mutex_new(v)          → Mutex  — create a mutex wrapping value `v`
//!   mutex_lock(m)         → value  — acquire lock, return wrapped value
//!   mutex_unlock(m)       → void   — release lock (no-op in interpreter)
//!   mutex_try_lock(m)     → Option — non-blocking; always Some in interpreter
//!   rwlock_new(v)         → RwLock — create an RwLock wrapping value `v`
//!   rwlock_read(l)        → value  — acquire shared read, return value
//!   rwlock_write(l)       → value  — acquire exclusive write, return value
//!   rwlock_unlock(l)      → void   — release lock (no-op in interpreter)
//!
//! ## Notes
//!
//! Both `Mutex` and `RwLock` are backed by a single-element `Value::Array`.
//! The lock/unlock operations are no-ops in the interpreter; the API matches
//! what a compiled backend would use with real OS primitives.

use crate::Value;

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Mutex builtins
// ---------------------------------------------------------------------------

/// `mutex_new(v) → Mutex` — wrap `v` in a new mutex.
pub(crate) fn builtin_mutex_new(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Array(vec![v.clone()])),
        _ => Err(format!(
            "mutex_new: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_lock(m) → value` — acquire the mutex and return the wrapped value.
/// In the interpreter this always succeeds immediately.
pub(crate) fn builtin_mutex_lock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("mutex_lock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_lock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_unlock(m) → void` — release the mutex. No-op in the interpreter.
pub(crate) fn builtin_mutex_unlock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(Value::Void),
        [_] => Err("mutex_unlock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_unlock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `mutex_try_lock(m) → Option<value>` — non-blocking acquire.
/// In the interpreter this always returns `Some(value)`.
pub(crate) fn builtin_mutex_try_lock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => {
            Ok(Value::Option(Some(Box::new(cells[0].clone()))))
        }
        [Value::Array(_)] => Ok(Value::Option(None)),
        [_] => Err("mutex_try_lock: argument is not a mutex".to_string()),
        _ => Err(format!(
            "mutex_try_lock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// RwLock builtins
// ---------------------------------------------------------------------------

/// `rwlock_new(v) → RwLock` — wrap `v` in a new read-write lock.
pub(crate) fn builtin_rwlock_new(args: &[Value]) -> RResult<Value> {
    match args {
        [v] => Ok(Value::Array(vec![v.clone()])),
        _ => Err(format!(
            "rwlock_new: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_read(l) → value` — acquire a shared read guard.
pub(crate) fn builtin_rwlock_read(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("rwlock_read: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_read: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_write(l) → value` — acquire an exclusive write guard.
pub(crate) fn builtin_rwlock_write(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(cells)] if !cells.is_empty() => Ok(cells[0].clone()),
        [_] => Err("rwlock_write: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_write: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `rwlock_unlock(l) → void` — release a read or write guard. No-op in interpreter.
pub(crate) fn builtin_rwlock_unlock(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::Array(_)] => Ok(Value::Void),
        [_] => Err("rwlock_unlock: argument is not an rwlock".to_string()),
        _ => Err(format!(
            "rwlock_unlock: expected 1 argument, got {}",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// Advisory type-check pass
// ---------------------------------------------------------------------------

/// No-op check: static analysis for mutex usage is handled by the existing
/// `deadlock_freedom` and `lock_priority` modules.
pub(crate) fn check(_program: &crate::Node, _source_path: &str) -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::run_program;

    fn run(src: &str) -> String {
        let r = run_program(src);
        assert!(r.ok, "program failed: {:?}", r.errors);
        r.stdout
    }

    #[test]
    fn mutex_new_and_lock() {
        let out = run(r#"
let m = mutex_new(42);
let v = mutex_lock(m);
println(to_string(v));
mutex_unlock(m);
"#);
        assert!(out.contains("42"), "got: {out:?}");
    }

    #[test]
    fn mutex_try_lock_returns_some() {
        let out = run(r#"
let m = mutex_new("hello");
let opt = mutex_try_lock(m);
println(to_string(is_some(opt)));
"#);
        assert!(out.contains("true"), "got: {out:?}");
    }

    #[test]
    fn rwlock_read_and_write() {
        let out = run(r#"
let l = rwlock_new(100);
let r = rwlock_read(l);
println(to_string(r));
let w = rwlock_write(l);
println(to_string(w));
rwlock_unlock(l);
"#);
        assert!(out.contains("100"), "got: {out:?}");
    }

    #[test]
    fn mutex_wraps_different_types() {
        let out = run(r#"
let m1 = mutex_new(true);
let m2 = mutex_new("world");
let v1 = mutex_lock(m1);
let v2 = mutex_lock(m2);
println(to_string(v1));
println(v2);
"#);
        assert!(out.contains("true"), "got: {out:?}");
        assert!(out.contains("world"), "got: {out:?}");
    }
}
