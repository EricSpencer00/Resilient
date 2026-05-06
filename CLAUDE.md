# CLAUDE.md — Resilient

Guidance for Claude Code when working in this repository. These rules
override default Claude Code behaviour. Human contributor instructions
(CONTRIBUTING.md, STABILITY.md) take precedence over this file.

## Agent execution style

**Do not ask for confirmation or approval at intermediate steps.** Pick
the best path forward and execute it. When facing a design choice, choose
the option that is most consistent with the existing codebase, most
correct, and most complete — then ship it. Only surface blockers that
genuinely cannot be resolved without human input (e.g., missing secrets,
conflicting requirements).

**Ship-to-merge, no review queue.** Once CI is green on a PR, it
auto-merges. There is no human review gate. The CI suite *is* the
gate — if it passes, the change ships. Build it accordingly: write
tests that prove the change is correct, lint clean, and let CI close
the loop.

**Be ambitious.** "Complex," "needs-design," and "blocked" are routing
hints, not stop signs. Most complex tickets decompose into 3–5 shippable
PRs once you start sketching. If the first PR you land is "scaffold the
new module + smoke test," that's progress — keep going. The default
posture is to *attempt* every open ticket; only fall back to single-line
stdlib additions when the design space genuinely needs human input
(see "Hard stops" below for the narrow list).

---

## What is this repo?

Resilient is a compiled, statically-typed language for safety-critical
embedded systems. The workspace contains:

| Crate | Purpose |
|---|---|
| `resilient/` | Compiler, CLI driver, REPL, JIT, LSP |
| `resilient-runtime/` | `#![no_std]` embedded runtime |
| `resilient-runtime-cortex-m-demo/` | Cortex-M4F cross-compile smoke test |
| `resilient-span/` | Source-span / diagnostic types |
| `playground/` | WASM-targeted web playground |
| `benchmarks/` | Performance benchmarks |
| `fuzz/` | Fuzz harnesses |

This is an **agent-native** project — AI contributors are first-class.
[GitHub Issues](https://github.com/EricSpencer00/Resilient/issues) are the
canonical source of work. Pick a ticket, claim it, ship it.

---

## Quick-start commands

```bash
# Build the compiler
cargo build --manifest-path resilient/Cargo.toml

# Run all compiler tests
cargo test --manifest-path resilient/Cargo.toml

# Run runtime tests
cargo test --manifest-path resilient-runtime/Cargo.toml

# Lint (must be clean before opening a PR)
cargo clippy --all-targets -- -D warnings

# Format
cargo fmt --all
```

Optional features: `--features z3` (SMT verifier), `--features lsp`,
`--features jit`, `--features ffi`.

---

## Ticket workflow — ship-to-merge

The path from picking a ticket to a merged commit is fully automated
once CI is green. There is no maintainer review step — **green CI is
the merge.** Your job is to make CI green.

1. **Pick a ticket.** Any open ticket is fair game — including ones
   tagged `complex`, `needs-design`, or `blocked`. The label tells you
   the *shape* of the work, not whether to attempt it:
   - `agent-ready` — single-PR, single-session. Default starting point.
   - `complex` — multi-PR scope. Break it into a sequence of shippable
     increments (see "Tackling complex tickets" below) and start with
     the first one.
   - `needs-design` — open design questions. Make the call yourself,
     write it up in the PR body, and ship. If the maintainer disagrees
     they'll revert or follow-up; that's cheaper than blocking.
   - `blocked` — has a stated prerequisite. Read the prereq; if you
     can also unblock it in a sibling PR, do that first. Otherwise
     pick a different ticket.
2. **Pre-dispatch overlap check.** Before creating your branch:
   ```bash
   agent-scripts/check-overlaps.sh resilient/src/main.rs resilient/src/typechecker.rs resilient/src/lexer_logos.rs
   ```
   If conflicts are reported, wait for those PRs to merge before starting.
3. **Claim the ticket.** Comment on the issue, then create a branch named
   `res-NNN-short-title`.
4. **Claim core files** immediately (before any edits):
   ```bash
   agent-scripts/claim-files.sh res-NNN-short-title resilient/src/main.rs resilient/src/typechecker.rs resilient/src/lexer_logos.rs
   ```
5. **Open a draft PR early** with `Closes #N` in the body — this signals
   the ticket is taken.
6. **Build, test, lint locally.** Match every CI gate (see "CI gates"
   below) before pushing. A red CI run wastes minutes you could spend
   shipping.
7. **Push to remote immediately after every commit.** Do not accumulate
   local commits.
8. **When everything is green locally, run:**
   ```bash
   agent-scripts/ready-or-bail.sh --pr N
   ```
   This runs the local guardrail, syncs your branch onto
   `agents/integration`, marks the PR ready, and applies the
   `integration-synced` label that releases the auto-merge gate.
   Do not call `gh pr ready` directly — the guardrail owns the
   draft-to-ready transition.
9. **Walk away.** Once the PR is ready + integration-synced, the
   `agent-auto-merge` workflow watches CI. As soon as every required
   check is `SUCCESS`, GitHub squashes and merges the PR
   (`gh pr merge --squash --auto --delete-branch`). The issue closes
   automatically; file claims release on merge.

If you spot CI fail-then-pass flakiness, open a follow-up ticket — do
not retry blindly.

Commit format: `RES-NNN: short description` (≤72 chars on the first line).
Include a `Co-Authored-By: Claude Sonnet 4.6 <noreply@anthropic.com>` trailer.

---

## Feature isolation pattern

**This is the single most important rule for parallel agent work.**

> **Heads up (RES-929):** historical references in this file say
> `src/main.rs` for the big core file. The actual file is now
> `src/lib.rs` (1.8 MB; `main.rs` is a 463-byte binary entry).
> The append-only allowlist in `agent-scripts/auto-resolve-extensions.sh`
> and `agent-scripts/sync-integration.sh` covers both.

Every language feature MUST follow this layout:

```
resilient/src/my_feature.rs   ← ALL feature logic lives here
resilient/src/lib.rs          ← minimal: token + AST variant + dispatch call (~5 lines total)
resilient/src/typechecker.rs  ← minimal: one function call in the <EXTENSION_PASSES> block
resilient/src/lexer_logos.rs  ← minimal: one token arm
```

### Core files have designated extension points

`lib.rs`, `typechecker.rs`, and `lexer_logos.rs` contain comment markers:

```rust
// <EXTENSION_TOKENS>    ← add Token variants here
// <EXTENSION_KEYWORDS>  ← add "keyword" => Token::X mappings here
// <EXTENSION_PASSES>    ← add check_my_feature(...) calls here
```

**Always add to the extension point block, never elsewhere in the file.**
These blocks are append-only — two agents adding to the same block will
produce a conflict that's trivially resolved by keeping all lines.

### What goes in `my_feature.rs` vs core files

| Element | Location |
|---|---|
| All feature logic (parser, type check, Z3 proofs) | `src/my_feature.rs` |
| Token enum variant | `main.rs` `<EXTENSION_TOKENS>` |
| Keyword → Token mapping | `main.rs` `<EXTENSION_KEYWORDS>` |
| Logos lexer token | `lexer_logos.rs` `<EXTENSION_TOKENS>` |
| Top-level check call | `typechecker.rs` `<EXTENSION_PASSES>` |
| AST node variant | `main.rs` `Node` enum — add to the end |

### Minimal main.rs touch example

```rust
// In Token enum — <EXTENSION_TOKENS> block:
/// RES-NNN: `my_keyword` — brief description.
MyKeyword,

// In keyword map — <EXTENSION_KEYWORDS> block:
"my_keyword" => Token::MyKeyword,

// In Node enum — at the end before the closing brace:
/// RES-NNN: MyFeature node.
MyFeatureNode { span: Span, ... },
```

### Minimal typechecker.rs touch

```rust
// In the <EXTENSION_PASSES> block:
crate::my_feature::check(program, source_path)?;
```

If you follow this pattern, two agents working in parallel will at most conflict on
the 3-line extension blocks — conflicts that are always safe to resolve by keeping both.

---

## Tackling complex tickets

Most "complex" tickets are not actually complex — they're just *large*.
The trick is to land work incrementally so each PR is a single coherent
story that CI can validate, and the next PR starts from a green baseline
instead of a half-built scaffold.

### Decomposition heuristics

| Ticket shape | Typical decomposition |
|---|---|
| Refactor (e.g., bin → lib split, file move, visibility audit) | (1) move/rename with `pub use` shims so nothing visible changes; (2) tighten visibility / migrate callers; (3) delete shims. Each step ships independently. |
| New language feature (sum types, generics, polymorphic Array) | (1) lexer + parser + AST node, accepting only the trivial case; (2) typechecker integration; (3) interpreter / VM / JIT codegen, one backend per PR; (4) Z3 mapping if applicable. Tests grow alongside. |
| New runtime primitive (actor, MMIO, interrupt) | (1) data model + `no_std` types; (2) host-side test harness; (3) embedded smoke test on Cortex-M; (4) docs + example. |
| Cross-cutting design lock-in (TLA+ V2 questions, semantic decisions) | (1) write the decision into a docs page with the alternatives considered; (2) follow up with code that enforces it. The doc PR alone unblocks downstream work. |

### When you decompose

- Open the **first** PR with a body that lists the planned next PRs.
  This is the design surface — anyone who disagrees can comment before
  you've sunk effort into PR #2.
- Each subsequent PR references the predecessor (`Built on #N`) so the
  chain is auditable in `gh pr list`.
- It's fine to leave a feature behind a `#[cfg(feature = "experimental_X")]`
  gate while landing the pieces. Strip the gate in the final PR.
- Don't let "I haven't finished the whole ticket" stop you from merging
  the parts that *are* finished. Each green PR is value; the ticket
  closes when the last PR lands.

### When the design genuinely needs the maintainer

Surface a *specific* question with a *specific* recommendation in the PR
body, not a vague "what should I do?" If the maintainer doesn't answer
within a session, ship your recommendation and let revert-if-wrong be
the disagreement protocol. The goal is a moving frontier, not perfect
upfront alignment.

---

## Agent autonomy — full discretion

You can ship without asking on any of these:

- Claim open GitHub Issues and implement them end-to-end — including
  ones tagged `complex`, `needs-design`, or `blocked`.
- Decompose a single ticket into multiple PRs (and open the chain
  yourself) when the work is too large for one diff.
- Land scaffolding / refactors / `pub use` shims that don't fully
  finish a ticket, as long as each PR ships a coherent green story.
- Open a "sibling" PR to unblock a prerequisite ticket so your main
  ticket stops being blocked.
- Refactor `main.rs` (35k+ lines) — including extracting modules,
  changing visibility, splitting the binary into a `[lib]` + `[[bin]]`,
  and rewriting whole subsystems — as long as tests stay green.
- Add new source files, tests, and `.expected.txt` golden sidecars.
- Fix compiler warnings and clippy lints anywhere in the codebase.
- Add or expand documentation (README, docs/, SYNTAX.md, LSP.md).
- Update `Cargo.toml` dependency versions (patch-level only).
- Open draft PRs and push to feature branches.
- Resolve merge conflicts on any PR branch — including checking out
  branches, editing conflicting files, and force-pushing to unblock
  stalled PRs.
- Mark a PR ready via `agent-scripts/ready-or-bail.sh` when the
  guardrail passes — auto-merge takes it from there.
- Post durable handoff comments on your PR with `agent-scripts/agent-handoff.sh`.

## Hard stops — these still need explicit human input

These are the only situations where an agent must NOT proceed
autonomously. They are deliberately narrow because the auto-merge
flow assumes everything else is recoverable from CI signal.

- **`unsafe` blocks** — see "Security rules" below.
- **Breaking changes to the stable language surface** — read STABILITY.md
  first; if your change touches anything in the "Stable" feature list,
  stop and surface the design decision.
- **Secrets / credentials** — never commit them; if you find one already
  committed, surface it to the maintainer.
- **Bypassing CI** — never use `--no-verify`, `--no-gpg-sign`, or skip
  hooks. The CI suite *is* the merge gate; circumventing it
  short-circuits the whole flow.
- **Force-pushing commits that are already merged** — once a commit
  lands on `main`, treat it as immutable.

Everything else — including modifying tests, bumping dependencies,
editing CI workflows — is on the table as long as you can justify it
in the PR body and CI stays green. The auto-merge gate trusts CI; you
should too.

---

## Test discipline (not approval — discipline)

Tests are the merge gate. Treat them accordingly:

- **A failing test is never a reason to weaken the test.** Fix the
  implementation. If the test was wrong, fix the test in the same PR
  with a clear "Test changes" section in the PR body explaining why
  the old assertion was incorrect.
- **Do not delete tests to make a PR green.** If the test is genuinely
  obsolete, the PR body must say so and link the obsoleting ticket.
- **Lowering an assertion = deleting the test.** Same rule.
- **Modify existing tests sparingly.** Most PRs add tests rather than
  edit them. If yours edits, flag it in the PR description under a
  **"Test changes"** section with a one-line rationale per test.
- **`#[cfg(test)]` modules, `.expected.txt` golden files, fuzz
  harnesses, and benchmark baselines** all count as tests.

The auto-merge workflow does not look at *which* tests changed — it
looks at whether they pass. The discipline above is what keeps tests
meaningful even though no human is gating each PR.

---

## Security rules

Resilient targets safety-critical embedded environments. Security discipline
is non-negotiable.

### No panics

- **`resilient-runtime/`**: zero panics in default (no_std) build. Use
  `Result`/`Option`. Every `unwrap()` or `expect()` is a bug.
- **`resilient/` parser and lexer**: all error paths must return a typed
  `Error` and surface a clean diagnostic. A panic in the compiler is a bug.
- Panics are acceptable only in test code and `main()` setup logic.

### `unsafe`

- Do not introduce new `unsafe` blocks without explicit justification in
  a code comment explaining the invariant that makes it sound.
- Any PR that adds or modifies `unsafe` must be flagged in the PR
  description. The auto-merge workflow does not block on unsafe, but
  the maintainer reads every `unsafe` diff post-merge — keep them clean
  and well-justified or expect a revert.

### `no_std` constraints (`resilient-runtime/`)

- Zero use of `std` types (`Vec`, `String`, `Box`, etc.) outside of
  `#[cfg(feature = "alloc")]` gates.
- No heap allocation in the default feature set.
- Cross-compile must pass for all three embedded targets CI checks:
  `thumbv7em-none-eabihf`, `thumbv6m-none-eabi`, `riscv32imac-unknown-none-elf`.

### Supply-chain hygiene

- Do not add new dependencies without a clear reason stated in the PR.
- Prefer in-tree implementations of small utilities over new crates.
- All new crates must appear in `Cargo.lock` before the PR merges (no
  floating version requirements).

### Secrets and credentials

- Never commit tokens, keys, or credentials. The `.gitignore` is not a
  safety net.
- If you see a potential secret in the codebase, flag it to the maintainer
  immediately rather than committing over it.

---

## Code standards

- `cargo fmt --all` must be clean.
- `cargo clippy --all-targets -- -D warnings` must be clean.
- No bare `println!` / `eprintln!` debug output in library code — use the
  diagnostic infrastructure.
- Diagnostics carry `line:col:` source positions.
- New built-in functions: add a doc-comment and a test.
- New language features: add an `.expected.txt` golden sidecar in
  `resilient/examples/`.

---

## CI gates (every gate is a merge gate)

These are the checks branch protection requires before auto-merge fires.
**Reproduce all of them locally before running `ready-or-bail.sh`** —
discovering a fail in CI wastes minutes per cycle.

| Check | Command |
|---|---|
| Build | `cargo build --locked` |
| Tests | `cargo test --locked` |
| Clippy | `cargo clippy --locked --all-targets -- -D warnings` |
| Format | `cargo fmt --check` |
| Z3 | `cargo test --features z3` |
| Embedded cross | `cargo build --target thumbv7em-none-eabihf` etc. |
| Size gate | `.text` ≤ 64 KiB for Cortex-M4F demo |
| Perf gate | `cargo bench` regression check |
| Fuzz | short fuzz run on changed harnesses |

Required-status-checks set on the `main` branch (these block auto-merge
if any are not `SUCCESS`):

- `build / test / clippy`
- `build / test with --features z3`
- `board hygiene`
- `resilient-runtime-cortex-m-demo (thumbv7em-none-eabihf)`
- `resilient-runtime (riscv32imac-unknown-none-elf)`
- `resilient-runtime (thumbv6m-none-eabi)`
- `cortex-m demo .text budget check`

The `diff-shape guardrail` reports overlaps but does not block merge —
overlaps on `main.rs` extension blocks are expected and are resolved
during integration sync.

---

## Auto-merge mechanics (reference)

Documented here so agents can debug a stuck PR without re-reading the
workflow file.

- **Trigger**: `.github/workflows/agent-auto-merge.yml` runs on
  `pull_request` (`labeled`, `ready_for_review`, `synchronize`),
  `check_suite: completed`, and on-demand via `workflow_dispatch`.
- **Preconditions** (all must hold):
  1. PR is **not draft** (set by `ready-or-bail.sh`).
  2. PR has the **`integration-synced` label** (applied by
     `sync-integration.sh` when the rebase onto `agents/integration`
     succeeds without conflicts outside the append-only allowlist).
  3. PR base is **`main`**.
  4. **Every required status check is `SUCCESS`** (the diff-shape
     guardrail's overlap sub-check is allowed to be `FAILURE`; everything
     else must be green).
- **Action**: workflow runs `gh pr merge $PR --squash --auto --delete-branch`.
  GitHub's server-side auto-merge then waits for any in-flight checks
  to settle and lands the squash merge.
- **Issue closure**: GitHub closes the issue when the squash commit
  containing `Closes #N` lands on `main`.
- **File-claim release**: `.github/workflows/release-file-claims.yml`
  fires on the merge commit and clears the agent's claim entries.

If your PR is ready + green but not merging, the most likely cause is
a missing `integration-synced` label. Re-run `ready-or-bail.sh` to
re-apply it.

---

## What not to do

- Do not create scratch planning files in the repo — keep design notes
  in the PR body or in `docs/` if they're durable. (For complex tickets,
  a design section in the PR body is *expected*, not a plan-doc.)
- Do not add comments that explain what code does — use well-named
  identifiers. Only add a comment when the *why* is non-obvious.
- Do not add error handling for impossible cases — trust internal invariants.
- Do not introduce backwards-compatibility shims for removed code,
  *except* during multi-PR refactors where a `pub use` shim keeps the
  intermediate PRs green. Strip it in the final PR.
- Do not half-implement a feature and leave an unscoped `TODO`. Either
  finish it, or open a follow-up ticket and reference it from the
  `TODO(RES-NNN)` comment.
- Do not wait on a human review that is not coming. Green CI is the
  merge — make CI green.
- Do not pre-emptively decline a ticket because it looks hard. Sketch
  the decomposition first; most "hard" tickets become "five medium PRs"
  once you do.
