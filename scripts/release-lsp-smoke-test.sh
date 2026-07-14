#!/usr/bin/env bash
# RES-4002: release smoke test for LSP-enabled `rz` binaries.
#
# Building with `--features lsp` says nothing about whether the
# resulting binary actually speaks the protocol correctly — a binary
# that merely links tower-lsp but panics/hangs on the very first
# request would still pass a plain `--help`/`--version` check, and
# nobody would notice until an editor integration silently failed to
# connect. This script starts `rz --lsp`, sends a minimal JSON-RPC
# `initialize` request over stdio using standard LSP `Content-Length`
# framing, and asserts the response contains a `capabilities` object
# before the harness kills the server.
#
# Usage:
#   scripts/release-lsp-smoke-test.sh <path-to-rz-binary>
set -euo pipefail

BIN="${1:?usage: release-lsp-smoke-test.sh <path-to-rz-binary>}"

[ -x "$BIN" ] || { echo "error: $BIN is not an executable file" >&2; exit 1; }

PYTHON="$(command -v python3 || command -v python)" || {
    echo "error: python3 not found on PATH (needed to speak LSP framing)" >&2
    exit 1
}

"$PYTHON" - "$BIN" <<'PYEOF'
import json
import subprocess
import sys

bin_path = sys.argv[1]

request = {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
        "processId": None,
        "rootUri": None,
        "capabilities": {},
    },
}


def frame(msg: dict) -> bytes:
    body = json.dumps(msg).encode("utf-8")
    header = f"Content-Length: {len(body)}\r\n\r\n".encode("ascii")
    return header + body


def read_message(stream, timeout_bytes=1 << 20):
    header = b""
    while b"\r\n\r\n" not in header:
        chunk = stream.read(1)
        if not chunk:
            raise RuntimeError("EOF before end of headers")
        header += chunk
        if len(header) > timeout_bytes:
            raise RuntimeError("header too large, likely not LSP framing")
    head, _, rest = header.partition(b"\r\n\r\n")
    length = None
    for line in head.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    if length is None:
        raise RuntimeError(f"no Content-Length header in: {head!r}")
    body = rest
    while len(body) < length:
        chunk = stream.read(length - len(body))
        if not chunk:
            raise RuntimeError("EOF before full body read")
        body += chunk
    return json.loads(body.decode("utf-8"))


proc = subprocess.Popen(
    [bin_path, "--lsp"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
)

try:
    proc.stdin.write(frame(request))
    proc.stdin.flush()
    response = read_message(proc.stdout)
except Exception as exc:  # noqa: BLE001 - want a clean release-smoke-test failure
    proc.kill()
    stderr = proc.stderr.read().decode("utf-8", errors="replace")
    print(f"FAIL: {exc}", file=sys.stderr)
    if stderr:
        print("--- stderr ---", file=sys.stderr)
        print(stderr, file=sys.stderr)
    sys.exit(1)
finally:
    if proc.poll() is None:
        proc.kill()
        proc.wait(timeout=5)

if response.get("id") != 1:
    print(f"FAIL: expected response id 1, got: {response}", file=sys.stderr)
    sys.exit(1)

result = response.get("result")
if not isinstance(result, dict) or "capabilities" not in result:
    print(f"FAIL: expected 'initialize' response with a 'capabilities' object, got: {response}", file=sys.stderr)
    sys.exit(1)

print("OK: rz --lsp answered 'initialize' with a capabilities object.")
print(json.dumps(result["capabilities"], indent=2))
PYEOF
