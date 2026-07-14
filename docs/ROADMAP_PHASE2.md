# Road to v1.0 — Phase 2+ Roadmap

> **Parent tracker:** [#3933](https://github.com/EricSpencer00/Resilient/issues/3933).
> This doc scopes the work *remaining* after the 2026-07-13/14 swarm, which landed
> ~47 PRs completing **Phase 0 (truth & soundness)** and most of **Phase 1 (prove it
> holds)**. It picks up at **Phase 2 (complete the language)** and runs through
> **Phase 4 (ship 1.0)**. Child tickets reference `#3933 · <EPIC>`.

## Where we are (entering Phase 2)

**Done in the swarm:** X1–X5 cross-cutting blockers; body-aware `ensures`; Z3 shipped +
static-linked + overflow-modeled; certificate corruption-proofing; the embedded pipeline
end-to-end and **CI-proven under QEMU** (`.rz` → `rz build` → `.rzbc` → no_std loader →
Cortex-M execution); VM/interpreter parity from a 14-example spot-check to a **~528-example
enforced corpus** (silent-wrongness denylist 150 → ~4); LSP-default binary; JIT transparent
VM-fallback; `rz fmt --check`; conformance suite 8 → 23; trait-bound enforcement (conservative);
all F-E3 design docs; ~370 builtins documented.

### 1.0 "definition of done" — gate status

| Gate | Status |
|---|---|
| Every Stable bullet has a 3-backend conformance test | 🟡 partial — suite exists (23 cases), needs full STABILITY.md coverage (F-E1) |
| Zero silent-wrongness on the stable surface | 🟢 near-complete — ~4 VM denylist entries + `#4041` runtime-contract parity remain |
| Default binary delivers core experience (Z3 + LSP) | 🟢 done for the primary target; other release targets tracked in `#3985` |
| No aspirational docs / nonexistent features | 🟡 mostly — **`#4025`** (`#[interrupt]` documented Stable but unimplemented) still open |
| `.rz` runs on an embedded target under QEMU in CI | 🟢 **done for the scalar subset**; `fn`/calls + interrupts extend it (D-E1 #6) |
| 10 `RES-350x` design tickets have merged docs | 🟢 done |
| semver + CHANGELOG + deprecation policy | 🟡 policy doc exists; release automation dry-run pending (F-E6) |

---

## Track A — Language Completeness *(the bulk of Phase 2; deep, serial `typechecker.rs` work)*

Most of this contends on `typechecker.rs`/`lib.rs`, so run these **one at a time** (not parallel).

### A-E2 · Generics completeness *(cont.)*
- [ ] Compound bounds `T: A + B` and where-clause propagation across nested generic calls
      (the `#4048`/#4049 increment deferred these — the arg's type is itself an unresolved param).
- [ ] Non-identifier callee resolution (method calls, closures) at bound-checked call sites.
- [ ] Const-generics minimal design + impl (`array<T, N>`), closing the `lib.rs` deferral.
- [ ] Differential monomorphization tests across all three backends.

### A-E3 · Trait system: associated types & trait objects
- [ ] Decide + document v1 scope for associated types (ship, or mark unsupported in the tier table).
- [ ] Typechecker projection resolution for `Self::Width` (parser + `AssociatedTypeDecl` exist; **zero
      enforcement today**).
- [ ] Trait-object / `dyn` dispatch: implement, or formally document static-dispatch-only for v1.

### A-E4 · Pattern-matching exhaustiveness *(cont.)*
- [ ] **`#4011`** — nested/payload pattern exhaustiveness (`Some(Shape::Circle(r))`-class) is
      unchecked → silent wrong value at runtime. Needs a decision-tree algorithm, not a hardening pass.
- [ ] Or-pattern / int-range exhaustiveness for payload enums; guard-clause interaction.

### A-E5 · Memory / ownership: region inference
- [ ] Region/lifetime **inference for unannotated code** (`region_inference::infer` is a documented
      no-op stub today; `MEMORY_MODEL.md`'s "Enforcement Reality Check" section flags the gap).
- [ ] Finish conditional-path use-after-move via Z3 fallback.

### A-E7 · Effect system: higher-order soundness
- [ ] Effect-polymorphic HOF signatures (`fn run<E>(f: () -> int ! E) -> int ! E`) — the `!` effect-arrow
      is **parsed-only** today (no unification). Corpus: a pure HOF called with an `io` callback must reject.

### A-E6 · Module & package system (language side)
- [ ] Complete module-path resolution; `pub use` re-export + glob-import coverage; circular-import
      diagnostics for cycles > 2 modules.

---

## Track B — Backends *(VM tail; can interleave with A since it's a different file set)*

### B-E3 · Remaining VM parity gaps *(each is compiler.rs/vm.rs — serialize)*
- [ ] **`#3993`** leftover — remaining `Unsupported` constructs (`break <expr>`, return-in-match-arm-
      expression-position, `??` null-coalescing operator, quantifier asserts).
- [ ] **`#3992`** leftover — static/namespaced/tuple-struct-constructor call-site lowering; fn-valued
      locals/closures captured across function/actor-spawn boundaries.
- [ ] **`#4017`** — VM CallMethod closures, StringBuilder write-back, mutual-recursion TCO.
- [ ] **`#4041`** — VM runtime `ensures`/`recovers_to` postcondition checking (in flight / just landed;
      verify parity with the interpreter's runtime contract enforcement).
- [ ] Direct-dispatch (`RESILIENT_DISPATCH=direct`) engine parity for `EnterLive`/static/etc. ops
      (currently returns `Unsupported`; not a CI path but should not silently diverge).

### B-E4 · JIT completeness *(cont.)*
- [ ] String literal/op + struct field access/construction in JIT lowering (i64-only today; VM-fallback
      covers correctness, but native coverage is the perf story).
- [ ] Wire a JIT differential pass for the supported subset; JIT startup-latency/memory benchmarks.

---

## Track C — Verification *(the differentiator; deep, z3-gated)*

- [ ] Wire `prove_overflow_safe` (BV64, shipped by C-E3) into the `requires`/`ensures` static path so
      contracts get overflow-safe checking (opt-in attribute vs default + the "LIA-provable but
      BV64-disprovable" diagnostic UX — needs a small design pass).
- [ ] C-E3 cont.: single non-recursive function-call inlining as an axiom; struct field access in
      `translate_int`/`translate_bool`; Real/float theory; recursion depth bounds.
- [ ] C-E4 · TLA+ Phase B: vendor `tla2tools.jar` in CI (tooling-blocked today, tracked by `#3930`);
      `@refines` parsing; narrow actor-subset → TLA+ exporter to make "translates to TLA+" literally true.

---

## Track D — Runtime & Embedded *(pipeline foundation done; extend coverage)*

### D-E1 · Embedded VM: beyond the scalar subset
- [ ] **`fn`/call support** in the no_std embedded VM (a call-frame stack) — the single biggest limiter;
      `rz build` currently rejects any program with functions.
- [ ] Fix the documented `Op::Return`-empty-stack divergence (needs a `Void` variant or static
      stack-depth analysis) flagged in `rzbc_emit.rs`.
- [ ] Array/heap types on-device behind an `alloc` gate (22 of 54 opcodes need `alloc`).
- [ ] `#[interrupt(...)]` lowering (`__resilient_isr_*`) end-to-end + QEMU interrupt-injection test —
      **coupled to `#4025`** (see human-decisions below).

### D-E2 · Board reference app *(unblocked once D-E1 has `fn` support)*
- [ ] Build `thermal_safety_cutoff.rz` via the real pipeline for Cortex-M4F; simulated ADC-in/PWM-out
      under QEMU; replace the "honest gap" paragraph in `REFERENCE_APP_THERMAL_CUTOFF.md`.

### D-E3 · stdlib portability enforcement
- [ ] Compiler lint when a Tier-2/3 builtin is reachable in a no_std/wasm32 target; graceful `Err`
      stubs for `file_meta`/`http`/`env`/`exec`/`tcp` on wasm32 (mirror the `file_io.rs` VFS pattern).

---

## Track E — Tooling & DX

- [ ] E-E2 · package-manager registry: `rz pkg update`, a minimal registry protocol (even a static JSON
      index) so `pkg add <name>` resolves without `git:`/`path:`, checksum verification.
- [ ] **E-E3 · vsce republish** — 🔴 **needs the maintainer** (external publish; see below).
- [ ] E-E4 · diagnostic error-code coverage: `E####` convention + a lint failing CI on a new codeless
      diagnostic; `rz explain E####`; auto-generate `docs/errors/*.md`.
- [ ] E-E7 · MCP hardening for public hosting: API-key auth + rate limiting on `/mcp/call`.

---

## Track F — Stability, CI & Release

- [ ] F-E1 · grow conformance coverage to **every** STABILITY.md Stable bullet across 3 backends;
      promote the suite to a required check.
- [ ] **`#4021`** · fix `ready-or-bail.sh`'s auto-`Closes` heuristic (it keeps trying to close the
      umbrella `#3933`). Quiet-window infra fix.
- [ ] **`#3976`** · keep `agent-scripts/file-claims.json` out of feature-branch PR diffs (serialization
      tarpit — every merge DIRTIES other open PRs). Quiet-window infra fix; edits shared scripts.
- [ ] F-E6 · release process: audit the `v1.5.x` tags vs `Cargo.toml 0.2.3`; dry-run the full release
      pipeline against a `v1.0.0-rc` tag; formalize the CHANGELOG.

---

## 🔴 Decisions that need the maintainer (do NOT proceed autonomously)

1. **E-E3 · VS Code Marketplace republish** (canonicalize `0.2.3`, wipe public `1.5.3`). Unpublishing/
   publishing a public listing is an external, irreversible action — the maintainer runs `vsce` themselves.
2. **`#4025` · `#[interrupt(name=...)]`** is documented **Stable** but the parser rejects it. Reclassifying
   it out of the Stable surface is a stability-policy hard-stop. Decide: **implement** it (D-E1 interrupt
   lowering) or **de-Stable** it (STABILITY.md edit). Blocks the "no aspirational features" 1.0 gate.

---

## Suggested Phase-2 sequencing

1. **Close the soundness tail first** (cheap, high-confidence): finish the VM parity gaps (`#3993`/`#3992`/
   `#4017`/`#4041`), then the nested-pattern exhaustiveness hole (`#4011`). This finishes the "zero
   silent-wrongness" gate.
2. **Land the two infra fixes** (`#4021`, `#3976`) in a quiet window — they tax every future swarm.
3. **Resolve the two maintainer decisions** (E-E3, `#4025`) — they unblock the "honest docs" + marketplace gates.
4. **Then the deep language work** (A-E3 → A-E5 → A-E7), serialized on `typechecker.rs`, one epic per PR-chain.
5. **Extend embedded** (D-E1 `fn` support → D-E2 board app) to widen the on-device story past scalars.
6. **Ship prep** (F-E1 full conformance → F-E6 release dry-run → tag `v1.0.0-rc`).

## Open follow-up issues (as of this writing)
`#4025`, `#4021`, `#4017`, `#4011`, `#3993`, `#3985`, `#3977`, `#3976` — plus `#3930` (TLA+ Phase B,
tooling-blocked) and the `#3987` D-E1 chain remainder.
