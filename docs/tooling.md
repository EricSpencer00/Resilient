---
title: Tooling Reference
parent: Language Reference
nav_order: 4
permalink: /tooling
---

# Tooling Reference
{: .no_toc }

Every tool shipped by the Resilient compiler binary and its
surrounding scripts, in one place. This is a reference page — for
introductions, see [Getting Started](getting-started) or the
[Tutorial](tutorial).
{: .fs-6 .fw-300 }

<details open markdown="block">
  <summary>Table of contents</summary>
  {: .text-delta }
- TOC
{:toc}
</details>

---

## Compiler execution modes

The `rz` binary is a driver that can run a source file under
three different backends.

| Mode | Flag | Status | Notes |
|------|------|--------|-------|
| Tree-walking interpreter | *(default)* | stable | Fastest to iterate on. Accepts every language feature. |
| Bytecode VM | `--vm` | stable | ~12x faster than the interpreter on `fib(25)`. Stack-based. |
| Cranelift JIT | `--jit` | stable subset | Requires `--features jit`. ~12x faster than the VM. |

```bash
rz prog.rz              # interpreter
rz --vm prog.rz         # bytecode VM
rz --jit prog.rz        # Cranelift JIT (built with --features jit)
```

The JIT backend only ships AST lowerings for the stable subset
documented in [Performance](performance). Features outside the
subset fall through to the interpreter at runtime rather than
erroring.

## Inspection

### `--dump-tokens <file>`

Prints the lexer's token stream as `line:col Kind("lexeme")` and
exits. Useful when a parser error points at a mystery token.
Honours the `logos-lexer` feature flag — same output either way.
Mutually exclusive with `--lsp`.

```bash
rz --dump-tokens examples/hello.rz
```

### `--dump-chunks <file>`

Compiles the program through the VM pipeline (including the
RES-172 peephole pass) and prints a stable-format disassembly of
every bytecode chunk — `main` plus each user function — with
constants, offsets, lines, opnames, and resolved jump targets.

```bash
rz --dump-chunks examples/hello.rz
```

Mutually exclusive with `--dump-tokens` and `--lsp`. The column
contract is documented at the top of `resilient/src/disasm.rs`;
external tools may parse it.

## Type checking

### `--typecheck <file>` (also `-t`)

Runs the static type checker. Clauses (`requires` / `ensures`,
array bounds, etc.) that are statically discharged are **elided at
runtime** — no runtime check is emitted. Clauses that the checker
cannot discharge fall through to their usual runtime enforcement.

```bash
rz --typecheck prog.rz
rz -t prog.rz
```

`--typecheck` is implied by `--emit-certificate`.

## Verification

### `--audit <file>`

Prints a human-readable report of which contract clauses were
proven statically vs left to runtime. Useful for understanding
what the verifier is (and isn't) doing on a given program.

```bash
rz --audit examples/sensor_monitor.rz
```

### `--emit-certificate <dir>`

For each contract obligation that Z3 discharges, writes a self-
contained SMT-LIB2 file so a downstream consumer can re-verify
the proof under their own solver without trusting the Resilient
binary. One file per obligation, named `<fn>__<kind>__<idx>.smt2`.
Implies `--typecheck`. Requires `--features z3`.

```bash
rz --emit-certificate ./certs examples/cert_demo.rz   # binary built with --features z3
```

Every run also writes a `manifest.json` index with per-obligation
SHA-256 and (when signed) Ed25519 signatures.

### `--sign-cert <key.pem>`

Signs the concatenated certificate payload with an Ed25519
private key, writing a 64-byte signature to `<dir>/cert.sig`.
Only meaningful when paired with `--emit-certificate`. The PEM
envelope format is documented in `resilient/src/cert_sign.rs`.

### `rz verify-cert <dir>`

Re-checks `<dir>/cert.sig` against the binary's embedded public
key (or a `--pubkey <path>` override). Exits 0 on match, 1 on
tamper / wrong key, 2 on usage error.

```bash
rz verify-cert ./certs
rz verify-cert ./certs --pubkey ./trusted-pub.pem
```

### `rz verify-all <dir>`

Walks `<dir>/manifest.json` and re-checks every obligation:
SHA-256 of the `.smt2` file, Ed25519 signature (if present), and
optionally re-runs Z3 on each certificate when `--z3` is passed
(requires the `z3` binary on `PATH`). Output is a one-row-per-
obligation table; exit 0 iff every checked cell passes.

```bash
rz verify-all ./certs
rz verify-all ./certs --z3
```

## REPL

Launched by running `rz` with no file argument.

```bash
rz                           # start REPL
rz --examples-dir ./ex       # override the `examples` command's search path
```

Built-in commands:

| Command | Purpose |
|---------|---------|
| `help` | Show help message. |
| `exit` | Exit the REPL. |
| `clear` | Clear the screen. |
| `examples` | List example snippets (or real files under `--examples-dir`). |
| `typecheck` | Toggle static type checking on/off for the session. |

History is persisted via `rustyline`. Multi-line input is supported.

## Language Server (LSP)

Opt-in; requires building with `--features lsp`.

```bash
cargo build --features lsp --release
rz --lsp
```

Speaks LSP over stdio. Shipped features:

- Diagnostics (parse errors, type errors, lint output)
- Hover (types, contracts)
- Go-to-definition (top-level declarations)
- Completion (builtins + top-level decls; RES-188)
- Semantic tokens (keyword / function / variable / parameter / type /
  string / number / comment / operator; see `sem_tok` in
  `resilient/src/main.rs`)

See [LSP / Editor Integration](lsp) for editor config examples.

## Formatter

### `rz fmt <file> [--in-place]`

Canonical source-code formatter. Parses the input, walks the AST,
and pretty-prints it in canonical style:

- 4-space indentation
- One space around binary operators
- Opening brace on the same line as the introducing construct
- No trailing whitespace
- Blank line between top-level declarations
- `requires` / `ensures` clauses indented under the function
  signature
- `live` blocks follow the same brace style

```bash
rz fmt src/main.rs              # print to stdout
rz fmt --in-place src/main.rs   # overwrite the file
```

Exit codes: `0` = formatted, `1` = parse errors (formatter refuses
to touch broken input), `2` = usage error.

**Known limitation.** The formatter is a structural round-trip.
Comments are not preserved today (the parser discards them). Run
`fmt` only on code you're willing to re-attach comments to by
hand. Comment-aware formatting is the next planned formatter
improvement.

## Package scaffolding

### `rz pkg init <name>`

Creates a new Resilient project layout in the current directory:

```
<name>/
  src/
    main.rs
  README.md
  .gitignore
```

```bash
rz pkg init my-proj
cd my-proj
rz src/main.rs
```

`rz pkg` is the umbrella for future package operations
(`pkg add`, `pkg build`, etc.); only `init` exists today.

## Fuzz testing

The `fuzz/` sibling crate carries [`cargo-fuzz`](https://rust-fuzz.github.io/book/cargo-fuzz.html)
targets. Two are shipped:

- `lex` — drives `Lexer::new` + `next_token` to EOF on arbitrary
  input. No panic paths in the lexer.
- `parse` — drives `parse(src)` and asserts the parser never
  panics regardless of input.

```bash
cd fuzz
cargo +nightly fuzz run lex
cargo +nightly fuzz run parse
```

Nightly Rust is required by `cargo-fuzz`. Corpus and crash seeds
live under `fuzz/corpus/<target>/` and `fuzz/artifacts/<target>/`.

## Benchmarking

Performance numbers are produced by the benchmark driver in
`benchmarks/`.

```bash
benchmarks/run.sh
```

The script runs `fib(25)` across every backend (interpreter, VM,
JIT, and the reference Rust / Python / Node / Lua / Ruby
implementations in `benchmarks/ref/`) and writes a Markdown table
to `benchmarks/RESULTS.md`. See [Performance](performance) for the
methodology and headline numbers.

## Reproducibility

### `--seed <u64>`

Pins the SplitMix64 PRNG used by `random_int` / `random_float` so
the same seed replays the same sequence. When `--seed` is not
passed the driver derives a seed from the monotonic clock and
echoes `seed=<N>` to stderr so a failing run can be replayed.

```bash
rz --seed 42 prog.rz
rz --seed=42 prog.rz
```

**Security note.** SplitMix64 is not cryptographic. Do not use
`random_*` for key material, nonces, or session tokens.

## Debugger — *future*

There is no standalone step-debugger today. The current debugging
aids are:

- `--dump-tokens` — inspect the lexer output
- `--dump-chunks` — inspect the compiled bytecode
- `println()` / `print()` in user code
- The LSP server's hover and diagnostics

A proper debugger (breakpoints, stepping, watch expressions, DAP
server) is tracked as a future deliverable. Until it lands,
bytecode-level inspection via `--dump-chunks` is the closest
equivalent.

## Profiler — *future*

There is no profiler today. Timing numbers come from the
benchmark driver (above) and from `--jit-cache-stats`, which
prints cumulative JIT cache (hits / misses / compiles) counters to
stderr on exit. A sampling profiler with a flame-graph emitter is
a future deliverable.

```bash
# What exists today:
rz --jit --jit-cache-stats prog.rz
```

## Test framework — *future*

Resilient programs express tests using ordinary `assert()` and
`assert(cond, msg)` calls — there is no `rz test`
subcommand yet. The assertion failure path includes the operand
values, which makes most failure modes easy to debug without a
dedicated framework.

```rust
fn main() {
    assert(add(2, 2) == 4, "add is broken");
}
main();
```

For CI, the model is the compiler's own test suite:

```bash
cd resilient
cargo test              # unit + integration tests
cargo test --features z3  # also exercises the SMT layer
```

A first-class `rz test` runner (test discovery, parallel
execution, JUnit output) is tracked as a future deliverable.

## Lint

### `rz lint <file>`

Parses the file and runs the starter linter (5 stable codes today;
see `resilient/src/lint.rs` for the full list). Supports
`// resilient: allow <code>` suppression comments.

```bash
rz lint src/main.rs
rz lint src/main.rs --deny L001
rz lint src/main.rs --allow L003
```

Exit codes: `0` = no diagnostics, `1` = warnings only, `2` = any
errors (either promoted via `--deny` or pre-existing errors).

---

## See also

- [Getting Started](getting-started) — install + first program
- [LSP / Editor Integration](lsp) — editor configuration
- [Performance](performance) — benchmark methodology and numbers
- [Certification and Safety Standards](certification) — how the
  verification tools map to specific regulatory objectives
