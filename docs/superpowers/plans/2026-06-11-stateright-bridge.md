# Stateright Bridge Implementation Plan
**For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
**Goal:** Add a feature-gated `rz stateright check <file.rz>` command that model-checks a narrow Resilient actor subset with Stateright.
**Architecture:** Reuse existing parser and actor AST, add a dedicated bridge module modeled after `tla_bridge`, and isolate all Stateright-specific translation logic inside that module. Keep the first version honest: support one integer actor state, straight-line receive handlers, and `always:` safety invariants only.
**Tech Stack:** Rust, Cargo features, Stateright crate, existing Resilient parser/CLI/diagnostics.
---

### Task 1: Wire Cargo feature and module
**Files:**
- Modify: `resilient/Cargo.toml`
- Modify: `resilient/src/lib.rs`

- [ ] **Step 1: Write failing integration-oriented compile expectation**
Document target compile surface:
```text
Expected: code references `stateright_bridge` from `lib.rs` and `stateright` feature from Cargo, but build/test fails until the module exists.
```

- [ ] **Step 2: Add feature-gated dependency and module declaration**
Add:
```toml
stateright = ["dep:stateright"]
```
and
```toml
stateright = { version = "...", optional = true }
```
plus a feature-gated module declaration and CLI dispatch hook in `lib.rs`.

- [ ] **Step 3: Run targeted compile/test command and verify failure moves to missing implementation**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge
```
Expected: compile fails because bridge functions/module implementation is incomplete, not because feature wiring is absent.

### Task 2: Add bridge CLI skeleton with tests first
**Files:**
- Create: `resilient/src/stateright_bridge.rs`

- [ ] **Step 1: Write failing dispatcher tests**
Add tests for:
```rust
dispatch_stateright_subcommand(&vec!["check".into(), "foo.rz".into()]).is_none()
dispatch_stateright_subcommand(&vec!["stateright".into(), "--help".into()]) == Some(0)
dispatch_stateright_subcommand(&vec!["stateright".into(), "check".into()]) == Some(1)
```

- [ ] **Step 2: Run targeted test and verify failure**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::stateright_help_returns_zero -- --exact
```
Expected: FAIL because the bridge module does not yet implement the dispatcher.

- [ ] **Step 3: Implement minimal CLI skeleton**
Add help text, `dispatch_stateright_subcommand`, `run_stateright_check`, and help-detection helpers following `tla_bridge`.

- [ ] **Step 4: Run targeted dispatcher tests and verify pass**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests -- --nocapture
```
Expected: PASS for CLI skeleton tests.

### Task 3: Add failing model-check tests for actor subset
**Files:**
- Modify: `resilient/src/stateright_bridge.rs`

- [ ] **Step 1: Write failing verification tests**
Add tests using inline source strings for:
```rust
actor Q {
  state: int = 0;
  always: state <= 2;
  receive push() requires state < 2 { self.state = self.state + 1; }
}
```
Expected clean result, and:
```rust
actor Q {
  state: int = 0;
  always: state <= 2;
  receive push() { self.state = self.state + 1; }
}
```
Expected violation result.

- [ ] **Step 2: Run targeted test and verify failure**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::unbounded_actor_reports_violation -- --exact
```
Expected: FAIL because translation/model checking is not implemented yet.

- [ ] **Step 3: Implement AST validation and translation**
Implement helpers that:
```rust
parse actor declaration from existing AST
enforce supported subset
translate `requires` and `always` expressions into a small evaluator
map each receive handler into a Stateright action
```

- [ ] **Step 4: Implement Stateright model runner**
Build a minimal `Model` with integer state, enumerate handler actions, and assert `always:` properties through Stateright.

- [ ] **Step 5: Run targeted tests and verify pass**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::bounded_actor_is_clean -- --exact
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::unbounded_actor_reports_violation -- --exact
```
Expected: PASS.

### Task 4: Add unsupported-shape coverage
**Files:**
- Modify: `resilient/src/stateright_bridge.rs`

- [ ] **Step 1: Write failing unsupported-shape test**
Use a handler body with control flow:
```rust
receive push() {
  if state < 2 { self.state = self.state + 1; }
}
```
Expected: unsupported diagnostic.

- [ ] **Step 2: Run targeted test and verify failure**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::control_flow_handler_is_unsupported -- --exact
```
Expected: FAIL before diagnostic path exists.

- [ ] **Step 3: Implement unsupported-shape diagnostics**
Return explicit Resilient-style errors with source location where available.

- [ ] **Step 4: Re-run targeted test and verify pass**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests::control_flow_handler_is_unsupported -- --exact
```
Expected: PASS.

### Task 5: Verify end-to-end
**Files:**
- Modify: `resilient/src/stateright_bridge.rs`
- Modify: `resilient/src/lib.rs`
- Modify: `resilient/Cargo.toml`

- [ ] **Step 1: Run focused feature test suite**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright stateright_bridge::tests -- --nocapture
```
Expected: PASS.

- [ ] **Step 2: Run broader crate tests with feature enabled**
Run:
```bash
cargo test --manifest-path resilient/Cargo.toml --features stateright
```
Expected: PASS.

- [ ] **Step 3: Manually verify CLI help**
Run:
```bash
cargo run --manifest-path resilient/Cargo.toml --features stateright -- stateright --help
```
Expected: help text for the new subcommand.

- [ ] **Step 4: Manually verify a clean and a violating inline/fixture case**
Run commands against one bounded and one violating source file.
Expected: one success diagnostic, one violation diagnostic.
