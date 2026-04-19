---
title: Contributing
nav_order: 13
permalink: /contributing
---

# Contributing to Resilient
{: .no_toc }

Whether you are a human engineer or an AI agent — welcome. Every
ticket landed, every test added, and every bug report filed makes
Resilient more reliable for everyone building safety-critical
systems.
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Full contributing guide

The canonical reference for contribution workflow, commit
conventions, code style, and the ticket system lives at
[`CONTRIBUTING.md`](https://github.com/EricSpencer00/Resilient/blob/main/CONTRIBUTING.md)
in the repository root. What follows is the quick-start version.

---

## Quick start

### 1. Fork and clone

```bash
# Fork on GitHub first, then:
git clone https://github.com/<your-username>/Resilient.git
cd Resilient
```

### 2. Verify everything passes

```bash
cd resilient
cargo test
```

All tests should be green before you write a single line. If
something is already broken, open an issue rather than working
around it.

### 3. Make your change

Create a branch, make focused commits, keep the test suite green.

```bash
git checkout -b my-fix
# ... edit files ...
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

### 4. Open a pull request

Push your branch and open a PR against `main`. Fill in the PR
template — a short description of *what* changed and *why* is
all that's needed. The CI workflow runs the full test matrix and
the perf gate automatically.

---

## The ticket system

Resilient tracks work in [`.board/`](https://github.com/EricSpencer00/Resilient/tree/main/.board)
— a plain-text ticket ledger checked into the repository. Each
ticket is a small Markdown file with a unique `RES-NNN` ID.

- **Claiming a ticket**: comment on the GitHub issue or PR that
  references the ticket, or add a `claimed_by` line to the file.
- **Landing a ticket**: your PR title or commit message should
  reference the ticket ID (e.g. `RES-042: add float division`).
- **Opening a ticket**: file a GitHub issue with a clear problem
  statement; a maintainer will assign it a `RES-NNN` ID.

---

## AI agents welcome

Resilient is intentionally designed with automated contributors
in mind. The ticket system is machine-readable, the test suite
is the authoritative acceptance signal, and CI is the gatekeeper.
Agents should follow the same workflow as humans: claim a ticket,
make a targeted change, pass `cargo test`, open a PR.

If you are an AI agent running in a sandboxed environment, the
minimum required commands are:

```bash
cargo test            # acceptance gate
cargo fmt             # style gate
cargo clippy          # lint gate
```

---

## Good first issues

Look for issues tagged
[`good first issue`](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aopen+label%3A%22good+first+issue%22)
on GitHub. These are scoped tasks with clear acceptance criteria
and no deep context dependencies.

---

## Where to get help

- **GitHub Discussions** — questions, design ideas, and
  feedback: [Discussions](https://github.com/EricSpencer00/Resilient/discussions)
- **GitHub Issues** — bug reports and concrete feature requests:
  [Issues](https://github.com/EricSpencer00/Resilient/issues)
- **PR comments** — for questions scoped to a specific change

Thank you for contributing. Every improvement — no matter how
small — moves Resilient closer to the goal: code that can be
trusted in the places where failure isn't an option.
