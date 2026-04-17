---
id: RES-173
title: `--dump-chunks` driver flag emits a human-readable VM disassembly
state: OPEN
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
