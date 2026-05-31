//! RES-2555: TCP/UDP socket builtins (std-only).
//!
//! TCP connections are represented as `Value::Struct { name: "TcpConn", fields: [("id", Int(N))] }`.
//! TCP listeners are `Value::Struct { name: "TcpListener", fields: [("id", Int(N))] }`.
//! UDP sockets are `Value::Struct { name: "UdpSocket", fields: [("id", Int(N))] }`.
//!
//! Handle ids are minted by monotonic counters and stored in thread-local registries
//! keyed by id. Closing a handle removes it from the registry so stale handles
//! surface `unknown handle` errors.

use crate::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

type RResult<T> = Result<T, String>;

// ---------------------------------------------------------------------------
// Handle registries
// ---------------------------------------------------------------------------

static NEXT_TCP_CONN: AtomicI64 = AtomicI64::new(1);
static NEXT_TCP_LISTENER: AtomicI64 = AtomicI64::new(1);
static NEXT_UDP: AtomicI64 = AtomicI64::new(1);

thread_local! {
    static TCP_CONNS: RefCell<HashMap<i64, TcpStream>> = RefCell::new(HashMap::new());
    static TCP_LISTENERS: RefCell<HashMap<i64, TcpListener>> = RefCell::new(HashMap::new());
    static UDP_SOCKETS: RefCell<HashMap<i64, UdpSocket>> = RefCell::new(HashMap::new());
}

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

fn tcp_conn_handle(id: i64) -> Value {
    Value::Struct {
        name: "TcpConn".to_string(),
        fields: vec![("id".to_string(), Value::Int(id))],
    }
}

fn tcp_listener_handle(id: i64) -> Value {
    Value::Struct {
        name: "TcpListener".to_string(),
        fields: vec![("id".to_string(), Value::Int(id))],
    }
}

fn udp_socket_handle(id: i64) -> Value {
    Value::Struct {
        name: "UdpSocket".to_string(),
        fields: vec![("id".to_string(), Value::Int(id))],
    }
}

fn extract_handle_id(v: &Value, kind: &str) -> RResult<i64> {
    match v {
        Value::Struct { name, fields } => {
            for (k, val) in fields {
                if k == "id" {
                    return match val {
                        Value::Int(i) => Ok(*i),
                        _ => Err(format!("{}: invalid handle id", kind)),
                    };
                }
            }
            Err(format!(
                "{}: expected {} handle, got struct '{}'",
                kind, kind, name
            ))
        }
        _ => Err(format!("{}: expected {} handle, got {:?}", kind, kind, v)),
    }
}

// ---------------------------------------------------------------------------
// TCP builtins
// ---------------------------------------------------------------------------

/// `tcp_connect(host: string, port: int) -> Result<TcpConn, string>`
///
/// Opens a TCP connection to `host:port`. Returns `Ok(TcpConn)` or `Err(msg)`.
pub(crate) fn builtin_tcp_connect(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(host), Value::Int(port)] => {
            let addr = format!("{}:{}", host, port);
            match TcpStream::connect(addr.as_str()) {
                Ok(stream) => {
                    let id = NEXT_TCP_CONN.fetch_add(1, Ordering::Relaxed);
                    TCP_CONNS.with(|r| r.borrow_mut().insert(id, stream));
                    Ok(ok(tcp_conn_handle(id)))
                }
                Err(e) => Ok(err(format!("tcp_connect: {}: {}", addr, e))),
            }
        }
        _ => Err(format!(
            "tcp_connect: expected (string host, int port), got {} arg(s)",
            args.len()
        )),
    }
}

/// `tcp_listen(host: string, port: int) -> Result<TcpListener, string>`
///
/// Binds a TCP listener on `host:port`. Returns `Ok(TcpListener)` or `Err(msg)`.
pub(crate) fn builtin_tcp_listen(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(host), Value::Int(port)] => {
            let addr = format!("{}:{}", host, port);
            match TcpListener::bind(addr.as_str()) {
                Ok(listener) => {
                    let id = NEXT_TCP_LISTENER.fetch_add(1, Ordering::Relaxed);
                    TCP_LISTENERS.with(|r| r.borrow_mut().insert(id, listener));
                    Ok(ok(tcp_listener_handle(id)))
                }
                Err(e) => Ok(err(format!("tcp_listen: {}: {}", addr, e))),
            }
        }
        _ => Err(format!(
            "tcp_listen: expected (string host, int port), got {} arg(s)",
            args.len()
        )),
    }
}

/// `tcp_accept(listener: TcpListener) -> Result<TcpConn, string>`
///
/// Accepts the next incoming connection on `listener`. Blocks until
/// a connection arrives. Returns `Ok(TcpConn)` or `Err(msg)`.
pub(crate) fn builtin_tcp_accept(args: &[Value]) -> RResult<Value> {
    match args {
        [handle] => {
            let id = extract_handle_id(handle, "tcp_accept")?;
            let result = TCP_LISTENERS.with(|r| {
                let borrow = r.borrow();
                match borrow.get(&id) {
                    Some(listener) => listener.accept().map_err(|e| format!("tcp_accept: {}", e)),
                    None => Err(format!(
                        "tcp_accept: unknown or closed listener handle {}",
                        id
                    )),
                }
            });
            match result {
                Ok((stream, _addr)) => {
                    let conn_id = NEXT_TCP_CONN.fetch_add(1, Ordering::Relaxed);
                    TCP_CONNS.with(|r| r.borrow_mut().insert(conn_id, stream));
                    Ok(ok(tcp_conn_handle(conn_id)))
                }
                Err(e) => Ok(err(e)),
            }
        }
        _ => Err(format!(
            "tcp_accept: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `tcp_read(conn: TcpConn, max_bytes: int) -> Result<string, string>`
///
/// Reads up to `max_bytes` from the connection. Returns the data as a
/// UTF-8 string (invalid bytes are replaced with U+FFFD). `Err` on
/// connection error.
pub(crate) fn builtin_tcp_read(args: &[Value]) -> RResult<Value> {
    match args {
        [handle, Value::Int(max_bytes)] => {
            let id = extract_handle_id(handle, "tcp_read")?;
            let cap = (*max_bytes).max(0) as usize;
            let result = TCP_CONNS.with(|r| {
                let mut borrow = r.borrow_mut();
                match borrow.get_mut(&id) {
                    Some(stream) => {
                        let mut buf = vec![0u8; cap];
                        stream
                            .read(&mut buf)
                            .map(|n| buf[..n].to_vec())
                            .map_err(|e| format!("tcp_read: {}", e))
                    }
                    None => Err(format!("tcp_read: unknown or closed connection {}", id)),
                }
            });
            match result {
                Ok(bytes) => Ok(ok(Value::String(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ))),
                Err(e) => Ok(err(e)),
            }
        }
        _ => Err(format!(
            "tcp_read: expected (TcpConn, int max_bytes), got {} arg(s)",
            args.len()
        )),
    }
}

/// `tcp_write(conn: TcpConn, data: string) -> Result<int, string>`
///
/// Writes `data` to the connection. Returns `Ok(bytes_written)` or `Err(msg)`.
pub(crate) fn builtin_tcp_write(args: &[Value]) -> RResult<Value> {
    match args {
        [handle, Value::String(data)] => {
            let id = extract_handle_id(handle, "tcp_write")?;
            let result = TCP_CONNS.with(|r| {
                let mut borrow = r.borrow_mut();
                match borrow.get_mut(&id) {
                    Some(stream) => stream
                        .write_all(data.as_bytes())
                        .map(|_| data.len())
                        .map_err(|e| format!("tcp_write: {}", e)),
                    None => Err(format!("tcp_write: unknown or closed connection {}", id)),
                }
            });
            match result {
                Ok(n) => Ok(ok(Value::Int(n as i64))),
                Err(e) => Ok(err(e)),
            }
        }
        _ => Err(format!(
            "tcp_write: expected (TcpConn, string data), got {} arg(s)",
            args.len()
        )),
    }
}

/// `tcp_close(conn: TcpConn) -> bool`
///
/// Closes the connection and removes it from the registry. Returns `true`
/// if the handle existed, `false` if it was already closed.
pub(crate) fn builtin_tcp_close(args: &[Value]) -> RResult<Value> {
    match args {
        [handle] => {
            let id = extract_handle_id(handle, "tcp_close")?;
            let existed = TCP_CONNS.with(|r| r.borrow_mut().remove(&id).is_some());
            Ok(Value::Bool(existed))
        }
        _ => Err(format!(
            "tcp_close: expected 1 argument, got {}",
            args.len()
        )),
    }
}

/// `tcp_set_timeout(conn: TcpConn, ms: int) -> bool`
///
/// Sets read/write timeout in milliseconds. `0` means no timeout.
pub(crate) fn builtin_tcp_set_timeout(args: &[Value]) -> RResult<Value> {
    match args {
        [handle, Value::Int(ms)] => {
            let id = extract_handle_id(handle, "tcp_set_timeout")?;
            let timeout = if *ms <= 0 {
                None
            } else {
                Some(Duration::from_millis(*ms as u64))
            };
            let ok = TCP_CONNS.with(|r| {
                let borrow = r.borrow();
                if let Some(stream) = borrow.get(&id) {
                    stream.set_read_timeout(timeout).is_ok()
                        && stream.set_write_timeout(timeout).is_ok()
                } else {
                    false
                }
            });
            Ok(Value::Bool(ok))
        }
        _ => Err(format!(
            "tcp_set_timeout: expected (TcpConn, int ms), got {} arg(s)",
            args.len()
        )),
    }
}

// ---------------------------------------------------------------------------
// UDP builtins
// ---------------------------------------------------------------------------

/// `udp_bind(host: string, port: int) -> Result<UdpSocket, string>`
///
/// Binds a UDP socket on `host:port`. Returns `Ok(UdpSocket)` or `Err(msg)`.
pub(crate) fn builtin_udp_bind(args: &[Value]) -> RResult<Value> {
    match args {
        [Value::String(host), Value::Int(port)] => {
            let addr = format!("{}:{}", host, port);
            match UdpSocket::bind(addr.as_str()) {
                Ok(socket) => {
                    let id = NEXT_UDP.fetch_add(1, Ordering::Relaxed);
                    UDP_SOCKETS.with(|r| r.borrow_mut().insert(id, socket));
                    Ok(ok(udp_socket_handle(id)))
                }
                Err(e) => Ok(err(format!("udp_bind: {}: {}", addr, e))),
            }
        }
        _ => Err(format!(
            "udp_bind: expected (string host, int port), got {} arg(s)",
            args.len()
        )),
    }
}

/// `udp_send_to(sock: UdpSocket, data: string, host: string, port: int) -> Result<int, string>`
///
/// Sends `data` to `host:port`. Returns `Ok(bytes_sent)` or `Err(msg)`.
pub(crate) fn builtin_udp_send_to(args: &[Value]) -> RResult<Value> {
    match args {
        [
            handle,
            Value::String(data),
            Value::String(host),
            Value::Int(port),
        ] => {
            let id = extract_handle_id(handle, "udp_send_to")?;
            let addr = format!("{}:{}", host, port);
            let result = UDP_SOCKETS.with(|r| {
                let borrow = r.borrow();
                match borrow.get(&id) {
                    Some(socket) => socket
                        .send_to(data.as_bytes(), addr.as_str())
                        .map_err(|e| format!("udp_send_to: {}", e)),
                    None => Err(format!("udp_send_to: unknown or closed socket {}", id)),
                }
            });
            match result {
                Ok(n) => Ok(ok(Value::Int(n as i64))),
                Err(e) => Ok(err(e)),
            }
        }
        _ => Err(format!(
            "udp_send_to: expected (UdpSocket, string data, string host, int port), got {} arg(s)",
            args.len()
        )),
    }
}

/// `udp_recv_from(sock: UdpSocket, max_bytes: int) -> Result<string, string>`
///
/// Receives up to `max_bytes`. Returns `Ok(data)` — the sender address
/// is not currently returned (add as a follow-up if needed).
pub(crate) fn builtin_udp_recv_from(args: &[Value]) -> RResult<Value> {
    match args {
        [handle, Value::Int(max_bytes)] => {
            let id = extract_handle_id(handle, "udp_recv_from")?;
            let cap = (*max_bytes).max(0) as usize;
            let result = UDP_SOCKETS.with(|r| {
                let borrow = r.borrow();
                match borrow.get(&id) {
                    Some(socket) => {
                        let mut buf = vec![0u8; cap];
                        socket
                            .recv_from(&mut buf)
                            .map(|(n, _src)| buf[..n].to_vec())
                            .map_err(|e| format!("udp_recv_from: {}", e))
                    }
                    None => Err(format!("udp_recv_from: unknown or closed socket {}", id)),
                }
            });
            match result {
                Ok(bytes) => Ok(ok(Value::String(
                    String::from_utf8_lossy(&bytes).into_owned(),
                ))),
                Err(e) => Ok(err(e)),
            }
        }
        _ => Err(format!(
            "udp_recv_from: expected (UdpSocket, int max_bytes), got {} arg(s)",
            args.len()
        )),
    }
}

/// `udp_close(sock: UdpSocket) -> bool`
///
/// Closes the UDP socket. Returns `true` if it existed.
pub(crate) fn builtin_udp_close(args: &[Value]) -> RResult<Value> {
    match args {
        [handle] => {
            let id = extract_handle_id(handle, "udp_close")?;
            let existed = UDP_SOCKETS.with(|r| r.borrow_mut().remove(&id).is_some());
            Ok(Value::Bool(existed))
        }
        _ => Err(format!(
            "udp_close: expected 1 argument, got {}",
            args.len()
        )),
    }
}
