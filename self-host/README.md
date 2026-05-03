# self-host/ — Resilient compiler in Resilient

Two artifacts live here, in increasing scope:

| Path | Ticket | Scope |
|---|---|---|
| `lex.rs` + `hello.tokens.txt` + `run.sh` | RES-196 | Prototype lexer covering a restricted Resilient subset; one snapshot input (`hello.rz`). Goal was "prove the language can express scanning at all." |
| `lexer.rz` + `lexer_tests/` + `lexer_check.sh` | RES-323 | Production-step-up lexer covering the verification surface (requires/ensures/invariant/assume/assert/...), block comments, escapes, hex/bin literals, and 2/3-char operators. |

## Running the RES-323 harness

```bash
# build the rz interpreter once
cargo build --manifest-path resilient/Cargo.toml

# run all snapshot tests
bash self-host/lexer_check.sh

# or against a release-built binary in a worktree:
RZ_BIN=/path/to/rz bash self-host/lexer_check.sh
```

The driver iterates `self-host/lexer_tests/*.rz`, runs the
self-hosted `lexer.rz` against each one with `SELF_HOST_INPUT`
pointing at the input, strips `seed=` / `Program executed
successfully` noise, and diffs the result against the committed
`*.expected.txt` snapshot. Exit non-zero on any diff.

## Running the RES-781 parity harness

```bash
cargo test --manifest-path resilient/Cargo.toml --test self_host_parity
```

This is the outer trust-loop gate. It walks
`self-host/parity_corpus/`, then:

- compares the Rust front end's `--dump-tokens` output against the
  self-hosted lexer token stream
- compares the Rust front end's `--dump-ast-json` output against the
  self-hosted parser JSON on curated success cases
- checks that a malformed corpus file fails in both front ends at the
  same source location

Failures name the corpus file plus the mismatching artifact (`tokens`,
`AST`, or parser-failure location), so regressions are easy to
triage.

## Adding a snapshot test

```bash
# 1. Drop a small Resilient input (it doesn't have to compile —
#    the lexer treats source as a token stream).
edit self-host/lexer_tests/my_case.rz

# 2. Capture the expected token output.
RZ_BIN=/path/to/rz \
  SELF_HOST_INPUT=self-host/lexer_tests/my_case.rz \
  /path/to/rz self-host/lexer.rz 2>/dev/null \
  | grep -v '^seed=' \
  | grep -v '^Program executed successfully$' \
  > self-host/lexer_tests/my_case.expected.txt

# 3. Re-run the harness to confirm green.
bash self-host/lexer_check.sh
```

Snapshots are part of the codebase — committing one is an explicit
agreement on what "correct" means for that input. If the snapshot
needs to change because the lexer's behavior intentionally
changed, regenerate it and call out the diff in the PR description
(test-protection policy applies — don't weaken assertions to make
the harness pass).

## Acceptance-criteria mapping (#115 / RES-323)

| Criterion | State |
|---|---|
| `self-host/lexer.rz` implements the complete Resilient lexer | 🟡 — covers significantly more than the RES-196 prototype (verification keywords, escapes, hex/bin, block comments, 3-char ops); full coverage matching every example in `resilient/examples/` is a follow-up |
| Test harness compares against the Rust frontend on a representative corpus | ✅ — `cargo test --manifest-path resilient/Cargo.toml --test self_host_parity` cross-checks lexer parity, parser AST parity, and one diagnostic-path case |
| `resilient run self-host/lexer.rz -- <input_file>` | ✅ — `SELF_HOST_INPUT` env var; CLI arg passing isn't a current language surface, env var is the idiomatic substitute |
| No new language features added to support this | ✅ — uses existing `file_read`, `env`, `is_ok`, `unwrap`, `split`, `push`, `len`, struct field access |

## What's next

1. **All-examples sweep.** Walk `resilient/examples/*.{rz,res}` and
   add a snapshot per file. Some inputs use string interpolation
   (`{...}` segments inside string literals) which the current
   self-hosted lexer does not yet decompose into nested expressions
   — that would be a bigger lexer extension.
2. **Multi-line strings.** If/when Resilient grows raw string
   literals, the `scan_string` function will need a separate path.
3. **Self-hosting parser** is tracked as the follow-up [#171](https://github.com/EricSpencer00/Resilient/issues/171)
   (RES-379). Sequenced after this lexer ships.
