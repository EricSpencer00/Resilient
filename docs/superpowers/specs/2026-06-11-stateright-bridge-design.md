# Stateright Bridge Design

## Goal

Add a first-class Stateright-backed verification path to Resilient via
`rz stateright check <file.rz>` without replacing existing Z3/TLA+
verification. The initial integration must be honest about scope and fail
cleanly on unsupported language features.

## Scope

This design covers a narrow MVP:

- Optional Cargo feature `stateright`
- New CLI subcommand `rz stateright check <file.rz>`
- Parsing and validation of Resilient actor programs
- Translation of a small actor subset into an in-process Stateright model
- Checking actor `always:` invariants under bounded exploration
- Resilient-style diagnostics for:
  - violated invariants
  - unsupported constructs
  - missing/invalid input

This design does not cover:

- General Resilient program execution in Stateright
- `eventually:` liveness translation
- `cluster_invariant` translation
- Explorer UI exposure
- Network semantics configuration
- Replacing existing actor Z3 verification

## Why This Shape

Resilient already has verification-specific command seams such as the TLA+
bridge. A Stateright bridge fits the same architecture: a separate subcommand
with explicit feature gating and focused diagnostics. That avoids coupling an
early integration to the full compiler pipeline.

The actor subset is the only credible starting point because Stateright is a
distributed/actor model checker, while most of Resilient is a general-purpose
language with embedded/runtime concerns that do not map naturally to Stateright.

## Supported Subset

The initial translator accepts actor declarations with:

- exactly one integer `state` field
- one or more `always:` clauses
- `receive` handlers whose bodies are straight-line updates to `self.state`
- integer literals, `state`, handler parameters, and `+`, `-`, comparison
  operators inside invariants and state updates

The command rejects programs using unsupported shapes, including:

- multiple actor declarations
- zero actors with `always:` clauses
- non-integer actor state
- control flow inside handlers
- unsupported expression forms in invariants or updates

## User-Facing Behavior

`rz stateright check file.rz`:

- parses the file
- locates the first actor declaration with `always:` clauses
- validates it against the supported subset
- runs a bounded Stateright exploration
- prints Resilient-style diagnostics

Success output:

```text
file.rz:0:0: info: Stateright model check completed — no invariant violations found.
```

Violation output:

```text
file.rz:0:0: error: Stateright found an invariant violation for actor `Q`: state <= 100
```

Unsupported subset output:

```text
file.rz:line:col: error: Stateright bridge currently supports only a single integer actor state and straight-line receive handlers.
```

## Architecture

### Cargo surface

Add an optional `stateright` feature in `resilient/Cargo.toml` that enables the
new dependency and compiles the bridge module.

### CLI integration

Follow the `tla_bridge` pattern:

- help request hook
- subcommand dispatcher
- dedicated help text
- explicit exit codes

### Translation layer

Create `resilient/src/stateright_bridge.rs` containing:

- CLI dispatch
- subset validation
- lightweight AST-to-model translation
- result rendering

The translator does not execute Resilient code generally. It only maps a small
symbolic state machine into a Stateright `Model`.

### Verification model

The initial model treats each receive handler as a possible action from the
current actor state. Preconditions come from `requires`; state transitions come
from symbolic evaluation of the handler body. `always:` clauses become
Stateright safety properties checked over reachable states.

This is intentionally narrower than the runtime actor semantics, but it yields a
real Stateright-backed invariant checker today.

## Testing Strategy

Add new tests only:

- CLI dispatch tests for `stateright` help and error cases
- positive unit test using `actor_queue_broken.rz`-style source that produces a
  violation
- positive unit test for a bounded actor that passes
- unsupported-shape unit test

## Risks

- Resilient actor syntax is richer than the first translator supports. The
  bridge must reject aggressively rather than approximate silently.
- Stateright APIs may require small adjustments depending on crate version. The
  integration should isolate that dependency inside the bridge module.

## Success Criteria

- `cargo test --manifest-path resilient/Cargo.toml --features stateright`
  passes
- `rz stateright check` exists behind the feature
- one real Resilient actor example can be model-checked end-to-end
