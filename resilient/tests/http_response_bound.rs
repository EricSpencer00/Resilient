//! RES-4839: response limits through the public CLI and a loopback peer.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const LIMIT: usize = 10 * 1024 * 1024;

fn request(response: Vec<u8>, post: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback peer");
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        socket
            .set_write_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).unwrap() > 0);
        // Over-limit clients may close the socket before the peer finishes.
        let _ = socket.write_all(&response);
    });
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "res_http_bound_{}_{}.rz",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let call = if post {
        format!("http_post(\"http://{address}/\", \"body\")")
    } else {
        format!("http_get(\"http://{address}/\")")
    };
    std::fs::write(&path, format!("let result = {call}\nmatch result {{\nOk(r) => println(len(r.body)),\nErr(e) => println(e),\n}}\n")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_rz"))
        .args(["--no-cache", "--feature", "std"])
        .arg(&path)
        .output()
        .unwrap();
    std::fs::remove_file(path).unwrap();
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn ordinary(size: usize) -> (Vec<u8>, usize) {
    let header = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n";
    let mut bytes = header.to_vec();
    bytes.resize(size, b'a');
    (bytes, size - header.len())
}

#[test]
fn exact_wire_limit_is_accepted() {
    let (bytes, body_len) = ordinary(LIMIT);
    assert!(
        request(bytes, false)
            .lines()
            .any(|line| line == body_len.to_string())
    );
}

#[test]
fn one_byte_over_wire_limit_is_rejected_for_get_and_post() {
    for post in [false, true] {
        assert!(
            request(ordinary(LIMIT + 1).0, post)
                .contains("HTTP response exceeds 10485760 byte limit")
        );
    }
}

#[test]
fn chunked_wire_boundary_preserves_decoding_and_rejects_over_limit() {
    let header = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
    let body_len = LIMIT - header.len() - 15;
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(format!("{body_len:x}\r\n").as_bytes());
    bytes.resize(bytes.len() + body_len, b'b');
    bytes.extend_from_slice(b"\r\n0\r\n\r\n");
    assert_eq!(bytes.len(), LIMIT);
    assert!(
        request(bytes.clone(), false)
            .lines()
            .any(|line| line == body_len.to_string())
    );
    bytes.push(b' ');
    assert!(request(bytes, false).contains("HTTP response exceeds 10485760 byte limit"));
}
