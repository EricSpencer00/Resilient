# Contributing to Resilient

Welcome! Resilient is an open project for safety-critical embedded systems. Contributions from both humans and AI agents are first-class. This guide covers the development workflow from setup through submission.

## Table of Contents

- [Development Environment Setup](#development-environment-setup)
- [Picking and Claiming a Ticket](#picking-and-claiming-a-ticket)
- [Commit Message Format](#commit-message-format)
- [Running Tests Locally](#running-tests-locally)
- [Code Style](#code-style)
- [Pull Request Checklist](#pull-request-checklist)
- [Golden Files and Expected Output](#golden-files-and-expected-output)
- [Releases](#releases)
- [Agent Contributors](#agent-contributors)

---

## Development Environment Setup

### Required

1. **Rust toolchain**: Install from [rustup.rs](https://rustup.rs/)
   - The project requires Rust 1.70+
   - Check: `rustc --version`

2. **Clone the repository**
   ```bash
   git clone https://github.com/EricSpencer00/Resilient.git
   cd Resilient
   ```

### Optional

- **Z3 (SMT solver)**: For symbolic contract verification
  - Install: `brew install z3` (macOS) or `apt-get install libz3-dev` (Linux)
  - Compile with: `cargo build --features z3`

- **LLVM**: For the JIT backend (advanced feature)
  - Install: `brew install llvm` (macOS) or `apt-get install llvm-dev` (Linux)
  - Compile with: `cargo build --features jit`

- **LSP support**: Language server for IDE integration
  - Compile with: `cargo build --features lsp`

---

## Picking and Claiming a Ticket

1. Browse [GitHub Issues](https://github.com/EricSpencer00/Resilient/issues)
2. Look for issues labeled `agent-ready`
3. Create a branch named `res-NNN-short-title` (e.g., `res-376-contributing-guide`)
   - `NNN` is the issue number
   - Use lowercase with hyphens for multi-word titles
4. Open a **draft PR** immediately with `Closes #NNN` in the description
   - The draft PR is the canonical claim signal
   - Convert to ready-for-review with `agent-scripts/ready-or-bail.sh --pr <number>` when you're done

---

## Commit Message Format

All commits must follow this format:

```
RES-NNN: short description (≤72 characters)

Optional longer explanation if the commit warrants it.
Wrap at ~72 characters.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

### Examples

```
RES-376: CONTRIBUTING.md documentation

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

```
RES-150: fix clippy warning in type checker

The pattern matching on Token could be simplified by using
unreachable_patterns. Applied the suggestion.

Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>
```

### Notes

- The issue number `NNN` is **required**
- First line must be ≤72 characters (git standard)
- Include the `Co-Authored-By` trailer for all commits
- **Push immediately** after each commit — don't accumulate local commits

---

## Running Tests Locally

### Full test suite

```bash
cargo test --manifest-path resilient/Cargo.toml
```

### Runtime tests

```bash
cargo test --manifest-path resilient-runtime/Cargo.toml
```

### Specific test

```bash
cargo test --manifest-path resilient/Cargo.toml <test_name>
```

### With Z3 (contract verification)

```bash
cargo test --features z3
```

### With JIT backend

```bash
cargo test --features jit
```

### All features

```bash
cargo test --all-features
```

---

## Code Style

### Format

All code must be formatted with `cargo fmt`:

```bash
cargo fmt --all
```

This is **required** before submitting a PR. CI will reject unformatted code.

### Clippy Lints

All compiler warnings must be clean:

```bash
cargo clippy --all-targets -- -D warnings
```

Common fixes:

- Use descriptive variable names instead of `_x`, `_temp`
- Replace `unwrap()` with `?` or proper error handling (except in tests)
- Use `matches!` macro for simple pattern matching
- Prefer `if let` over `match` with a single arm

### Panics

- **`resilient/` (compiler)**: Zero panics except in `main()` setup and tests
  - Use `Result`/`Option` for error handling
  - Parser and type checker must return typed errors
- **`resilient-runtime/`**: Zero panics in default (no_std) build
  - Every `unwrap()` and `expect()` is a bug
  - Use `Result`/`Option` exclusively
- **Tests**: Panics are acceptable in test code

### Comments

Add comments only when the **why** is non-obvious. Don't explain what the code does — use well-named identifiers instead.

Example (bad):

```rust
// Increment x by 1
x = x + 1;
```

Example (good):

```rust
// Align buffer to 8-byte boundary for DMA.
x = (x + 7) & !7;
```

---

## Pull Request Checklist

Before requesting review, ensure:

- [ ] **Branch is up-to-date** with `main`
  ```bash
  git fetch origin
  git rebase origin/main
  ```

- [ ] **All tests pass locally**
  ```bash
  cargo test --manifest-path resilient/Cargo.toml
  cargo test --manifest-path resilient-runtime/Cargo.toml
  ```

- [ ] **Code is formatted**
  ```bash
  cargo fmt --all
  ```

- [ ] **Clippy is clean**
  ```bash
  cargo clippy --all-targets -- -D warnings
  ```

- [ ] **PR title and description are clear**
  - Title: `RES-NNN: short description`
  - Body includes what changed and why

- [ ] **All CI jobs are green**
  - The PR will show CI status; all checks must pass
  - Do not request review while CI is still running

- [ ] **Commits are in order**
  - Each commit has a clear message in the `RES-NNN:` format
  - History is clean (no fixup commits left behind)

---

## Diff-shape guardrail

CI runs [`agent-scripts/verify-scope.sh`](./agent-scripts/verify-scope.sh)
on every PR. It blocks the harm modes that human review would otherwise
catch, while letting mechanical refactors (renames, path updates) through.

**What's blocked, no exceptions:**

- Deletion of any test file under `resilient/tests/`,
  `resilient-runtime/tests/`, or `fuzz/fuzz_targets/`.
- Any modification or deletion of an `.expected.txt` golden sidecar.
- A test diff that **removes more `#[test]` / `#[should_panic]` /
  `assert!` / `assert_eq!` / `assert_ne!` / `panic!` lines than it
  adds** (i.e. tests can't get weaker without an explicit OK).
- New `unsafe` blocks anywhere in the tree.
- Deletion of any `.github/workflows/*` file.
- A workflow diff that adds `if: false`, `continue-on-error: true`,
  `--no-verify`, or `permissions: write-all`.
- A workflow diff that drops the count of top-level `jobs.<name>:`
  entries (i.e. jobs can't be silently removed).
- More than 60 files changed in one PR.
- Major / minor version bumps in `Cargo.lock`.

**What's allowed:**

- Mechanical edits to existing test files — renaming an env var,
  updating a path, adapting to a public-API change. The assertion
  count rule above is what catches the actual weakening pattern.
- Workflow path / env / version edits when no jobs are removed and no
  bypass patterns are introduced.

If you need to legitimately weaken or remove a test (the tested
behaviour intentionally changed), call it out in a `## Test changes`
section in the PR description with a one-line rationale per test.
The maintainer will admin-merge if the rationale is sound.

---

## Golden Files and Expected Output

When a compiler change intentionally alters output (new language features, refactored error messages), you must update the golden `.expected.txt` files.

### Finding golden files

Golden files live alongside their test inputs in `resilient/examples/`:

```
resilient/examples/
├── feature_name.rz          # Input source
└── feature_name.expected.txt # Expected output
```

### Regenerating golden files

1. Run the test to see the actual output:
   ```bash
   cargo test --manifest-path resilient/Cargo.toml <test_name> -- --nocapture
   ```

2. Review the diff carefully to ensure it's correct

3. Update the golden file with the new output:
   ```bash
   cargo test --manifest-path resilient/Cargo.toml <test_name> -- --nocapture | tail -n +2 > resilient/examples/feature_name.expected.txt
   ```

4. Re-run the test to confirm it passes:
   ```bash
   cargo test --manifest-path resilient/Cargo.toml <test_name>
   ```

### In your PR

When you modify golden files:

1. Call it out explicitly in the PR description under a **"Test changes"** section with a one-line rationale per file:

   ```markdown
   ## Test changes

   - `feature_name.expected.txt`: Updated output for new language feature
   - `error_case.expected.txt`: Improved error message formatting
   ```

2. Do **not** delete tests to make a PR green — fix the code instead
3. Lowering or removing an assertion in a test requires the same approval as modifying the test

---

## Agent Contributors

### Live Ticket Flow

Resilient tracks active work in GitHub Issues and pull requests. The
`agent-ready` issue template is the live queue.

- **OPEN**: Ticket is available; nobody is actively working on it
- **CLAIMED**: An agent has opened a draft PR with `Closes #NNN`
- **DONE**: PR is merged; the issue closes automatically when you add
  `Closes #NNN` in the PR body

### Workflow

1. **Pick** an `agent-ready` issue.
2. **Create a branch** named `res-NNN-short-title`.
3. **Open a draft PR** immediately with `Closes #NNN` in the body.
4. **Push after every commit** — don't accumulate local commits.
5. **Mark ready for review** with `agent-scripts/ready-or-bail.sh --pr <number>` once the implementation and CI are green.
6. **Monitor for feedback** via the PR comment subscription.
7. Merge is automatic once CI is green and the PR has been synced; the issue closes.

### Legacy Local Ledger

The `.board/` tree and the `scripts/new-ticket.sh` /
`scripts/check-ticket-ids.sh` helpers remain in-tree for archive
maintenance and historical migration work. They are not the live queue
and should not be used to start new work unless you are explicitly
auditing or migrating historical tickets.

### Special Notes

- **Test protection**: Modifying existing tests requires maintainer approval (see [CLAUDE.md](./CLAUDE.md) for details)
- **Security**: Changes to `unsafe` blocks or breaking language features require explicit review
- **Dependencies**: Patch-level Cargo.toml updates are free; major/minor require approval

---

## Releases

Releases are automated. The shipped artifact is the **`rz`** CLI binary,
packaged as a per-platform `.tar.gz` and attached to a GitHub Release.

### Cutting a release (maintainers only)

1. Make sure `main` is green and the version in
   [`resilient/Cargo.toml`](./resilient/Cargo.toml) reflects what
   you're about to ship (bump it in a separate commit if needed).
2. Push a SemVer tag to `main`:
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
3. The [`release.yml`](./.github/workflows/release.yml) workflow fires:
   - Builds `rz` for **four** platforms in parallel:
     - `x86_64-unknown-linux-gnu` (native)
     - `aarch64-unknown-linux-gnu` (via [`cross`](https://github.com/cross-rs/cross))
     - `x86_64-apple-darwin`
     - `aarch64-apple-darwin`
   - Strips each binary, packages it as `rz-<tag>-<target>.tar.gz`,
     and uploads it as a workflow artifact.
   - Creates a GitHub Release with auto-generated notes (commits
     since the previous tag) and attaches all four archives.

The archive layout is flat — `rz` extracts directly into the
caller's current directory — so [`scripts/install.sh`](./scripts/install.sh)
can stream a tarball straight into `$PREFIX/bin`.

### Dry-running without a tag

`workflow_dispatch` is enabled on the release workflow:

1. Open the [Actions → release](https://github.com/EricSpencer00/Resilient/actions/workflows/release.yml) page.
2. Click **Run workflow** and pick a branch.
3. Build artifacts upload but the `release` job skips (it's
   guarded on `startsWith(github.ref, 'refs/tags/')`).

Use this to confirm a cross-compile change works before tagging.

### What's not in scope

- crates.io publishing (the package is `resilient`, not `rz`; we
  may publish later but not in the same workflow).
- Windows binaries.
- Code signing / notarization (macOS will Gatekeeper-warn on
  unsigned downloads — `xattr -d com.apple.quarantine ./rz` is
  the user-side workaround until we add signing).

---

## Questions or Blockers?

- For setup issues, open a GitHub Discussion or issue
- For questions about a specific ticket, comment on the issue
- For security concerns, reach out to the maintainers privately

Thank you for contributing to Resilient! 🚀
