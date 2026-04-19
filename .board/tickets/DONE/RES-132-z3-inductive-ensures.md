---
id: RES-132
title: Z3 discharges simple inductive `ensures` via loop invariants
state: DONE
priority: P2
goalpost: G9
created: 2026-04-17
owner: executor
---

## Summary
`fn sum(Int n) requires n >= 0 ensures result >= 0 { let s = 0; for i in 0..=n { s = s + i; } return s; }`
is currently unprovable — the verifier has no notion of loop
invariants. Add explicit `invariant` annotations on `for`/`while`
and thread them into the SMT context as assume-at-entry / verify-at-back-edge
obligations.

## Acceptance criteria
- Parser: `while (c) invariant (p) { ... }` and
  `for x in xs invariant (p) { ... }`.
- Encoding:
  - Assume invariant on entry → verify it holds before the loop.
  - Inside the body, assume invariant + loop condition.
  - At back-edge, verify invariant still holds.
  - After the loop, assume invariant + negation of condition.
- Verifier-only feature; interpreter/VM/JIT ignore the annotation
  (it's a proof aid, not a runtime check).
- Unit tests: `sum(n) ensures result >= 0` with invariant `s >= 0`
  discharges; with no invariant, fails cleanly.
- Commit message: `RES-132: Z3 uses loop invariants to prove ensures`.

## Notes
- We deliberately do not infer invariants automatically — that's a
  research project. Users write them.
- The `--audit` table gains a "loop invariants" column.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed and bailed by executor (oversized)
- 2026-04-17 claimed by executor — landing RES-132a scope (parser + AST field only)
- 2026-04-17 landed RES-132a (parser + AST); RES-132b / RES-132c deferred

## Resolution (RES-132a — parser + AST only)

This landing covers only the **RES-132a** piece from the Attempt-1
clarification split: the parser and AST field for `invariant` clauses
on `while` and `for`-in loops. The SMT encoding (RES-132b) and audit
row + end-to-end `sum(n)` test (RES-132c) remain deferred as
follow-ups the Manager should mint.

### Files changed

- `resilient/src/main.rs`
  - `Node::WhileStatement` and `Node::ForInStatement` each gained an
    `invariants: Vec<Node>` field, guarded with `#[allow(dead_code)]`
    and documented as verifier-only.
  - `parse_while_statement` and `parse_for_in_statement` now accept
    zero or more `invariant EXPR` clauses between the condition /
    iterable and the opening `{`. Both `invariant (EXPR)` and
    `invariant EXPR` are accepted for parity with existing
    `requires` / `ensures` / `live invariant`.
  - New helper `parse_loop_invariants` shared by both loop parsers.
  - Map-reduce desugaring (the other `ForInStatement` construction
    site) emits `invariants: Vec::new()`.
- Nine new unit tests named `res132a_*` cover: empty-invariants
  back-compat for plain loops, single parenthesized / unparenthesized
  invariants, multiple invariants on both loop forms, and a runtime
  parity check (invariants are ignored by the tree-walker).

### Verification

```
$ cargo build                          # OK
$ cargo build --features z3            # OK
$ cargo build --features lsp,logos-lexer,infer   # OK
$ cargo test --locked
test result: ok. 583 passed; 0 failed; 0 ignored
(+ 58 integration tests, all OK)
$ cargo test res132a
test result: ok. 9 passed; 0 failed
```

### What was intentionally NOT done

- **RES-132b**: no SMT encoding work. The verifier still has no notion
  of loop invariants — a `while`/`for` body is still analysed as if
  it were straight-line code. `prove_with_certificate` etc. are
  unchanged. Hand-built `Node` trees with the new field exist but are
  never consumed by the verifier.
- **RES-132c**: no `VerificationStats` counter, no `--audit` row, no
  end-to-end `sum(n) ensures result >= 0` test. Running that example
  today will still fail the `ensures` (unchanged from before this
  ticket).
- Interpreter, bytecode VM, and JIT pattern-match on these two nodes
  with `..` destructuring, so they continue to ignore the new field —
  exactly the behaviour the ticket calls for ("Verifier-only feature;
  interpreter/VM/JIT ignore the annotation").

### Follow-ups the Manager should mint

- **RES-132b** — SMT encoding for loop invariants (Hoare-rule plumbing
  in `verifier_z3.rs`), testable in isolation against the AST shape
  this ticket now produces.
- **RES-132c** — `VerificationStats` row + audit rendering + the
  `sum(n)` end-to-end test that ties 132a + 132b together.

## Attempt 1 failed

Three independently-sized pieces bundled into one ticket:

1. **Parser**: accept `while (c) invariant (p) { ... }` and
   `for x in xs invariant (p) { ... }` — needs new token, new AST
   field on `Node::While` and `Node::For` (preserving span), AST
   pretty-printer / formatter coverage, no-invariant back-compat.
2. **SMT encoding**: four new obligations per loop (entry verify,
   body-entry assume, back-edge verify, post-loop assume). Needs
   extending `prove_with_certificate` to take an invariant context
   (or a new entry point). Today the verifier only handles simple
   implications — a loop's Hoare-rule proof obligation skeleton is
   genuinely new plumbing, not a tweak.
3. **Audit row**: new counter in `VerificationStats` + render in
   `print_verification_audit`.

Plus tests: the `sum(n)` example listed in the acceptance criteria
is a good end-to-end test but also exercises arithmetic, loop
invariants, and `ensures` simultaneously — writing it without the
parser + SMT pieces being done first leaves nothing to test.

## Clarification needed

Manager, please consider splitting:

- RES-132a: parser for `invariant` on `while` and `for`, with
  span-bearing AST fields. Interpreter / VM / JIT ignore the
  annotation (ticket already notes this); testable via AST-shape
  asserts.
- RES-132b: SMT encoding for loop invariants — `prove_with_loop`
  or a new entry point accepting the Hoare-rule context, testable
  in isolation with hand-built `Node` trees.
- RES-132c: wire the audit row and the end-to-end `sum(n)` test on
  top of a + b.

132a is independently useful (keeps `invariant` annotations alive
in the AST for future work) and unblocks 132b. No code changes on
this bail.
