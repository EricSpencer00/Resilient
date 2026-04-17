---
id: RES-071
title: G19 export SMT verification certificate
state: DONE
priority: P1
goalpost: G19
created: 2026-04-17
owner: executor
---

## Summary
RES-067 wired Z3 in. RES-068 elides runtime `requires` checks for fully
proven functions. The next G19 step is to make those proofs reproducible
by a third party: when the verifier discharges a contract, dump the
SMT-LIB2 query (and Z3's UNSAT result) to disk so a downstream consumer
can re-run the proof under their own solver and confirm the result
without trusting our binary. This is "proof-carrying assertions" in
practice.

## Acceptance criteria
- New CLI flag `--emit-certificate <DIR>` on the resilient driver.
- When set, every successful Z3 verification writes one file per proven
  obligation: `<DIR>/<function-name>__<requires-or-ensures>__<idx>.smt2`,
  containing (a) the full `(set-logic ...)` preamble, (b) every `(declare-fun)`
  for free variables, (c) every `(assert)` for assumptions, (d) the negated
  goal `(assert (not <goal>))`, and (e) `(check-sat)` followed by a comment
  `; expected: unsat`.
- The emitted file, fed to `z3 -smt2 path.smt2`, prints `unsat` on stdout.
  Add a smoke test in `resilient/tests/` that runs Z3 on a generated
  certificate and asserts `unsat` (skip the test cleanly if `z3` is not on
  PATH — gate with `which::which("z3").is_ok()`).
- An end-to-end test compiles `resilient/examples/contracts_demo.res` (or
  the closest existing example with `requires` clauses), emits certificates,
  and confirms at least one .smt2 file is produced.
- Doc snippet in `README.md` under a new "Verification certificates" section
  explaining how to re-verify with stock Z3.
- `cargo build`, `cargo test`, `cargo clippy -- -D warnings` all pass.
- Commit message: `RES-071: --emit-certificate dumps re-verifiable SMT proofs`.

## Notes
- Verifier code lives in `resilient/src/verifier_z3.rs` (217 lines).
- The current verifier already builds a Z3 `Solver`; SMT2 text can be
  obtained via `solver.to_smt2()` in z3-rs ≥ 0.12. Confirm the API on the
  pinned version in `Cargo.toml`.
- Don't gate the smoke test on Z3 being installed in CI — make it skip
  cleanly so contributors without Z3 still get green.

## Log
- 2026-04-17 created by manager
- 2026-04-17 acceptance criteria filled in by manager
- 2026-04-17 executor landed:
  - `verifier_z3::prove_with_certificate` returns the SMT-LIB2 dump
    alongside the verdict; `prove` is now a thin wrapper. Built the
    certificate by hand (declare-const for every Int identifier in the
    AST, pin call-site bindings via `(assert (= NAME VAL))`, then
    `(assert (not <goal>))` followed by `(check-sat)`) — independent of
    z3-rs's internal `Solver::to_smt2()` so the file is portable to
    stock Z3.
  - `typechecker::CapturedCertificate { fn_name, kind, idx, smt2 }`
    accumulator on `TypeChecker`; both Z3 callsites (decl-level and
    call-site-discharge) push when proof succeeds.
  - Driver: `--emit-certificate <DIR>` (also `=DIR`) flag; implies
    `--typecheck`. After typecheck, writes one `.smt2` per cert as
    `<fn>__<kind>__<idx>.smt2` and prints a cyan summary line.
  - `examples/cert_demo.rs` declares `requires x + 0 == x` (cheap
    folder gives up on the free var; Z3 proves universally).
  - Smoke test `emit_certificate_writes_reverifiable_smt2` (gated on
    `--features z3`) runs the demo, asserts the .smt2 exists, then
    re-runs stock `z3 -smt2` if the binary is on PATH and asserts the
    output contains `unsat`. Skips re-verify cleanly if z3 is absent.
  - `README.md` gains a "Verification certificates" section under the
    SMT-backed verification block.
- 2026-04-17 verification: default `cargo build/test/clippy -- -D warnings` clean
  (152 tests). With `--features z3`: 161 tests, clippy clean. Stock Z3
  manual round-trip: `z3 -smt2 ./certs/ident_round__decl__0.smt2` →
  `unsat` (proof confirmed).
