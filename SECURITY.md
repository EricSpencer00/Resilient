# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| `main`  | ✓ (rolling) |

Resilient is pre-1.0 software. Only the `main` branch receives security fixes.

## Reporting a Vulnerability

**Please do not file public GitHub Issues for security vulnerabilities.**

Email [ericspencer1450@gmail.com](mailto:ericspencer1450@gmail.com) with:

- A description of the vulnerability and its potential impact
- Steps to reproduce or a proof-of-concept
- Any suggested mitigations you have in mind

You can expect an acknowledgment within 72 hours and a status update within 7 days.

## Scope

The Resilient compiler, runtime, and tooling are the primary scope. The embedded
`resilient-runtime` crate targets `no_std` environments where memory-safety
guarantees are especially critical — reports in that area are particularly welcome.

## Out of Scope

- Vulnerabilities in third-party dependencies (report upstream)
- Issues in example programs that do not affect the compiler or runtime
