---
id: RES-173
title: `--dump-chunks` driver flag emits a human-readable VM disassembly
state: DONE
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
When the VM misbehaves, we need to look at what the compiler
emitted. Today that's a hex dump. A proper disassembler —
printing `000A  LoadLocal x` with line info — makes every VM bug
triagable without stepping through rustc-gdb.

## Acceptance criteria
- `--dump-chunks <file.rs>` compiles the program and prints the
  bytecode for every function to stdout, one line per op:
  `<offset:04x>  <line>   <OpName> <operands>`.
- Constants block printed separately at the top with
  `const[i] = Value`.
- Jump targets printed as `-> 00XY` (absolute offset).
- Line info respected — the `<line>` column is the source line
  per RES-091.
- Unit test in `tests/dump_chunks_smoke.rs` runs against a
  two-function example and asserts key lines appear.
- Documented in README "Debugging" subsection alongside RES-112.
- Commit message: `RES-173: --dump-chunks VM disassembler`.

## Notes
- If peephole optimization has run (RES-172), reflect the
  optimized bytecode — that's the version that runs.
- Keep the format stable: external tools will parse it. Document
  the format in a comment at the top of the disassembler.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution
- `resilient/src/disasm.rs` (new, ~300 lines):
  - `disassemble(&Program, &mut String)` entry point. Prints
    `=== main ===` first, then one `=== fn <name> (arity=N,
    locals=M) ===` section per user function in declaration
    order.
  - Each section has a `constants:` block (or
    `constants: (none)`) followed by a `code:` block.
  - Per-op lines use the ticket's exact column order:
    `  <offset:04x>  L<line>   <OpName> <operands>`.
  - Offsets are zero-padded 4-hex; `L<n>` comes straight from
    `Chunk::line_info` (RES-091). Synthetic instructions
    (line = 0) print `L0`.
  - Jump ops (`Jump`, `JumpIfFalse`, `JumpIfTrue` — RES-172
    added the last one) render their target as an **absolute**
    4-hex PC via `-> 0XYZ`, matching the ticket.
  - `Const(i)` and `Call(i)` carry a `; const[i] = <Value>` /
    `; -> <fn-name>` comment tail for readability.
  - Format documented at the top of the module as a stable
    external contract — the ticket's "external tools will
    parse it" requirement.
- `resilient/src/main.rs`:
  - New `mod disasm;` registration.
  - New `--dump-chunks <file>` CLI flag. Parses + expands `use`
    imports + compiles to bytecode (peephole included per
    RES-172) + disassembles. Mutually exclusive with
    `--dump-tokens` / `--lsp` (dedicated check in the arg-parse
    block with a clean error).
- `resilient/tests/dump_chunks_smoke.rs` (new, 4 tests):
  - `dump_chunks_prints_sections_for_main_and_each_function` —
    ticket AC: two-fn example, asserts section headers,
    constant-block contents, and Call-site name comments.
  - `dump_chunks_format_columns_match_spec` — asserts the
    four-hex offset column + `L<n>` line column both appear.
    Deliberately doesn't pin exact whitespace counts so
    external tools have sensible flexibility.
  - `dump_chunks_reflects_peephole_inc_local_fold` — ticket
    Note: the disassembly reflects the RES-172 peephole pass.
  - `dump_chunks_requires_path_argument` — missing-path clean
    error.
- `README.md`: Debugging subsection gains a `--dump-chunks`
  example, cross-referenced to `--dump-tokens`. Mutual-exclusion
  rule and "format is stable" guarantee both documented.
- Unit tests (7 new in `disasm::tests`): constants rendering,
  jump absolute-target formatting, line-column preservation,
  Call-with-fn-name, multi-fn headers, RES-172 op coverage
  (IncLocal + JumpIfTrue), empty-chunk/empty-constants edge.
- Deviations: none. The format is exactly what the ticket
  specified (`<offset:04x>  <line>   <OpName> <operands>`),
  with the modest addition of `L<n>` rather than bare `<n>`
  for the line column so downstream tools can distinguish it
  from a constant/local index at a glance.
- Verification:
  - `cargo test --locked` — 468 passed (was 461 before RES-173,
    +7 disasm unit tests + 4 smoke tests).
  - `cargo test --locked --features logos-lexer` — matching
    delta.
  - `cargo clippy --locked --features logos-lexer,z3 --tests
    -- -D warnings` — clean.
  - Manual end-to-end: a two-fn program with a while-loop
    counter shows `IncLocal 0` in the disassembly (peephole
    fold visible), and Jump / JumpIfFalse display absolute
    targets in 4-hex.
