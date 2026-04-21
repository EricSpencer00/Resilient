---
title: Community & Open Source
nav_order: 14
permalink: /community
---

# Community & Open Source
{: .no_toc }

Resilient is free and open source software, built in public, one
ticket at a time.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## License

Resilient is released under the **MIT License**.

```
MIT License

Copyright (c) 2024 Eric Spencer

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

Full license text is in
[`LICENSE`](https://github.com/EricSpencer00/Resilient/blob/main/LICENSE)
at the repository root.

---

## GitHub repository

**[github.com/EricSpencer00/Resilient](https://github.com/EricSpencer00/Resilient)**

The repository contains:

| Path | Contents |
|------|----------|
| `resilient/` | Compiler and runtime source (Rust) |
| `docs/` | This documentation site (Jekyll) |
| `ROADMAP.md` | Goalpost ladder (G1–G20+) |
| `examples/` | Example `.res` programs |
| `resilient-runtime/` | `#![no_std]` runtime crate |

---

## Filing issues

Found a bug? Have a feature idea? Open an issue on GitHub:

1. Go to [Issues](https://github.com/EricSpencer00/Resilient/issues)
2. Click **New issue**
3. Choose the appropriate template (bug report, feature request,
   or blank)
4. Include a minimal reproducible example for bugs — the smaller
   the better

For compiler crashes, please include:
- The Resilient source that triggered the crash
- The full error output (`RUST_BACKTRACE=1 resilient your_file.rs`)
- Your OS and `cargo --version`

---

## Contributing

See the [Contributing](contributing) page for the full
workflow — fork, clone, `cargo test`, open PR.

The short version:
- All contributions go through GitHub pull requests
- The CI workflow (tests + fmt + clippy + perf gate) is the
  acceptance bar
- Work is tracked through
  [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
  with `RES-NNN` identifiers

---

## AI agents welcome

Resilient is designed with automated contributors in mind. The
ticket system is machine-readable plain text, the test suite
gives a deterministic pass/fail signal, and the contribution
workflow requires no human-only interaction beyond opening a PR.

Agents of any kind — code generation assistants, autonomous
coding agents, or CI bots — are welcome to:

- Pick up a [`good first issue`](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
- Submit a PR that passes `cargo test && cargo fmt --check && cargo clippy`
- Report bugs by opening a GitHub issue with a reproducible example

The same quality bar applies to everyone: green tests, clean
diffs, and a clear explanation of what changed and why.

---

## Roadmap and vision

Resilient is being built incrementally through the ticket system.
The current focus is on:

- Expanding JIT coverage (while loops, closures, structs)
- Growing the error code registry
- Strengthening the Z3 contract prover
- Improving the LSP experience

Follow along in [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
or watch the repository on GitHub to stay current.
