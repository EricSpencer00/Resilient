# Security Policy

## Supported Versions

Resilient is pre-1.0 and under active development. Security fixes are applied to the `main` branch only.

## Reporting a Vulnerability

**Please do not report security vulnerabilities via public GitHub issues.**

Email **ericspencer1450@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested mitigations

You should receive a response within 48 hours. If you do not, please follow up to ensure the message was received.

## Scope

Areas of particular interest for security reports:

- **FFI trampoline** (`resilient/src/ffi.rs`) — memory safety, arbitrary code execution via crafted `.so` files
- **Parser / lexer** — denial of service via crafted input, panics
- **`unsafe` blocks** — soundness violations
- **File I/O builtins** (`file_read` / `file_write`) — path traversal
- **Compiler-emitted certificates** — signature forgery, hash collision in manifest

## Security Model

Resilient programs run with the ambient authority of the host process. There is no sandboxing of the interpreter or VM. Do not run untrusted Resilient programs without an OS-level sandbox (e.g., container, seccomp, chroot).

The `resilient-runtime` embedded crate has no file I/O or network surface and is not in scope for most vulnerabilities.
