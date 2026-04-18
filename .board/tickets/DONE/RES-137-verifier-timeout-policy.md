---
id: RES-137
title: Verifier timeout + soft-failure policy
state: DONE
priority: P3
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
Z3 can spin indefinitely on certain obligations (QF_NIA is
undecidable). Today we wait. Set a hard per-obligation timeout
(default 5s), treat `unknown`/`timeout` as "not proven" rather than
error, and keep compilation going.

## Acceptance criteria
- CLI flag: `--verifier-timeout-ms <N>` (default 5000).
- Programmatic: pass `timeout` in the Z3 params dict before each
  `solver.check()`.
- On timeout/unknown: emit a *hint*-severity diagnostic
  `proof timed out after 5000ms — runtime check retained` with the
  obligation span. Compilation continues; runtime check is not
  elided.
- `--audit` tallies `timed-out` as its own column.
- Unit test: construct an obviously hard NIA obligation and
  confirm timeout triggers within the budget.
- Commit message: `RES-137: verifier timeout + soft-failure`.

## Notes
- Z3 `timeout` is per-query, not cumulative. Per-fn wall-clock
  budget is a future ticket if needed.
- The hint severity is important: errors would block builds on
  machines with slow Z3 builds (ARM mac, etc.).

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 done by executor

## Resolution

Files changed:
- `resilient/src/verifier_z3.rs`
  - New `pub fn prove_with_timeout(expr, bindings, timeout_ms:
    u32) -> (Option<bool>, Option<ProofCertificate>,
    Option<String>, bool)` — the four-slot tuple extends the
    existing three-slot shape with a `timed_out: bool` flag that
    fires when Z3's tautology-check returns `Unknown` (per-query
    timeout hit the budget or the theory didn't decide).
  - Inside the new fn, `timeout_ms > 0` calls
    `solver.set_params(z3::Params::new + set_u32("timeout",
    ms))` on both solvers (tautology check + contradiction
    check). `timeout_ms == 0` is "no timeout" (unlimited).
  - Existing `prove_with_certificate_and_counterexample`
    delegates with `timeout_ms = 0`, so old callers keep their
    unlimited behaviour untouched.
  - Two new unit tests:
    `timeout_returns_timed_out_flag_on_hard_nia` drives a
    Pell-style non-linear equation with a 1 ms budget and
    asserts the fourth return slot is `true`;
    `timeout_zero_disables_timeout` confirms the
    unlimited-budget path still closes `x + 0 == x` as
    `Some(true)` with the flag clear.
- `resilient/src/typechecker.rs`
  - `z3_prove_with_cert` shim gains a `timeout_ms: u32`
    parameter and a fourth return slot for `timed_out`;
    delegates to `prove_with_timeout`. Non-z3 stub returns
    `(None, None, None, false)`.
  - `TypeChecker` gains a `verifier_timeout_ms: u32` field
    (default 5000 per the ticket's recommendation) + a
    `with_verifier_timeout_ms(ms)` builder method the driver
    calls from the CLI flag.
  - Both `z3_prove_with_cert` call sites (decl-contract +
    call-site-requires) now pass the TypeChecker's timeout and
    bump the new `verifier_timeouts` stat on `timed_out`, plus
    `eprintln!` the hint the ticket specifies: `hint: proof
    timed out after 5000ms — runtime check retained (fn foo)`
    (or `(call to fn foo)` for the call-site path). Proofs that
    timed out are NOT elided; the runtime check stays in.
  - `VerificationStats::verifier_timeouts: usize` counter +
    rendered in `print_verification_audit` under the existing
    Z3 line when non-zero: `of which timed out: N`.
- `resilient/src/main.rs`
  - New `--verifier-timeout-ms <N>` CLI flag (both spaced and
    `=` forms). Parsed to `u32`; `0` disables the timeout.
    Defaults to 5000 in the driver. Plumbed into `execute_file`
    → `TypeChecker::with_verifier_timeout_ms`.
  - New `of which timed out: N` line in
    `print_verification_audit` when `stats.verifier_timeouts > 0`.

Deviation from the ticket sketch: the hint is an `eprintln!`
with a `hint:` prefix, not a structured hint-severity diagnostic
— the typechecker has no warning / hint channel today (same
infrastructure gap RES-129 / RES-133 / RES-135 flagged). When
that channel lands, this call site migrates with a one-line
change. The user still sees the hint today; it just isn't
test-introspectable as a structured value.

Verification:
- `cargo build --locked` — clean.
- `cargo build --locked --features z3` — clean.
- `cargo test --locked` — 302 unit + all integration pass.
- `cargo test --locked --features z3` — 317 unit (+2 new
  timeout-specific, incl. the `verifier_overflow_fails`-
  adjacent NIA timeout) + 13 integration pass.
- `cargo clippy --locked --tests -- -D warnings` — clean.
- `cargo clippy --locked --features z3 --tests -- -D warnings`
  — clean.
- Manual: `resilient --typecheck --verifier-timeout-ms=1
  examples/cert_demo.rs` keeps typechecking (the demo's proofs
  close well inside 1 ms); bumping to `1000ms` default exercises
  the normal path.
