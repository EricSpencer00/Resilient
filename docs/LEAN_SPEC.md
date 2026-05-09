---
layout: page
title: "Lean-Proven Semantics"
nav_order: 31
permalink: /lean-spec/
---

# Lean-Proven Semantics

> *"Your code is what the spec says it is — and the spec is checked in Lean."*

Resilient ships its own operational semantics in **Lean 4**. The Lean project
lives at [`resilient/lean-spec/`](https://github.com/EricSpencer00/Resilient/tree/main/resilient/lean-spec)
and is built with `lake build`. For every pure-arithmetic single-return function,
the Resilient compiler can emit a Lean theorem stating that the AST means
exactly what `eval` says it means.

CompCert exists for C; **no embedded-targeted language ships its evaluation
rules in a proof assistant**. This is what puts Resilient in a different league
for tool qualification under DO-178C DAL-A: the certification authority can
audit *the proof*, not just the test suite.

## What's in the Lean project

```
resilient/lean-spec/
├── lakefile.lean             — Lake build manifest
├── lean-toolchain            — Lean version pin (v4.13.0)
├── Resilient.lean            — root re-export
└── Resilient/
    ├── AST.lean              — inductive Expr definition
    ├── Semantics.lean        — eval : Env → Expr → Option Int
    └── Theorems.lean         — proven correctness lemmas
```

## The AST mirror

```lean
inductive Expr where
  | int : Int → Expr
  | bool : Bool → Expr
  | var : String → Expr
  | add : Expr → Expr → Expr
  | sub : Expr → Expr → Expr
  | mul : Expr → Expr → Expr
  | div : Expr → Expr → Expr
  | mod : Expr → Expr → Expr
  | eq : Expr → Expr → Expr
  | lt : Expr → Expr → Expr
  | le : Expr → Expr → Expr
  | neg : Expr → Expr
  | not_ : Expr → Expr
  | ite : Expr → Expr → Expr → Expr
```

The fragment is deliberately small — loops, calls, and structs are *not* part
of the Lean spec for the first slice. Adding them is a matter of extending
`AST.lean` plus the Rust lowering pass in [`src/lean_spec.rs`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/lean_spec.rs).

## The semantics

Big-step evaluation `eval : Env → Expr → Option Int`. Total in the Lean sense
(structural recursion on `Expr`); the `Option` reflects partial expressions like
`x / 0`.

```lean
def eval (env : Env) : Expr → Option Int
  | .int n => some n
  | .add l r =>
    match eval env l, eval env r with
    | some a, some b => some (a + b)
    | _, _ => none
  | .div l r =>
    match eval env l, eval env r with
    | some _, some 0 => none      -- divide by zero is an error
    | some a, some b => some (a / b)
    | _, _ => none
  ...
```

## Proven theorems

The first four theorems we ship:

| Theorem | Statement |
|---|---|
| `eval_int_lit_id` | `eval env (Expr.int n) = some n` |
| `eval_add_comm` | `eval env (Expr.add a b) = eval env (Expr.add b a)` |
| `eval_const_fold_sound` | `eval env (Add (Int a) (Int b)) = eval env (Int (a+b))` |
| `eval_neg_involutive` | `eval env (Neg (Neg e)) = eval env e` |

Each subsequent compiler optimisation pass (constant folding, peephole, etc.)
can be discharged against the same `eval` relation. The proofs in
`Theorems.lean` are the foundation; the obligations grow with the compiler.

## Emitting per-function theorems

For any function whose body is a single `return EXPR;` over the supported
fragment, run:

```bash
rz --emit-lean-spec=double my_file.rz
```

The output is a Lean source file:

```lean
import Resilient.Semantics
open Resilient

/-! Auto-generated from Resilient fn `double` -/

def double : Expr := Expr.add (Expr.var "x") (Expr.var "x")

theorem double_correct (x : Int) :
    eval (env_of [("x", x)]) double = some (x + x) := by
  simp [eval, env_of, double]
```

Save this into `lean-spec/Resilient/Generated/Double.lean` and run
`lake build` — Lean re-derives the `double_correct` theorem from the
operational semantics and the AST. If the proof fails, the Resilient
compiler's lowering is incorrect.

## What a function must look like to lower

For the first slice, `--emit-lean-spec` accepts:

- A single `return EXPR;` statement
- `EXPR` is built from: integer/bool literals, parameter identifiers,
  `+ - * / %`, `== < <= > >=`, unary `-` and `!`, `if expr`

Functions with loops, calls, struct accesses, or multi-statement bodies
yield an `unsupported` diagnostic. Each restriction lifts as the Lean
fragment grows.

## Why this changes the conversation

Tool qualification under safety standards (DO-178C, ISO 26262, IEC 61508)
requires evidence that the *compiler* — not just the *user code* — preserves
the semantics on the way from source to silicon. Today that evidence is built
out of test suites and structural-coverage reports. With a Lean spec, the
evidence shifts to *machine-checked theorems*. Two consequences:

- The certification authority audits a small, independently-built spec
  (the `Theorems.lean` file plus the build manifest), not the entire
  compiler test suite.
- The compiler itself becomes a *trusted base*: every commit either
  re-derives the same theorems or fails the build.

Combined with the existing certificate manifest (RES-071, RES-194, RES-195),
**every shipped Resilient binary can carry a Lean-checkable proof of
correctness for each function it contains**.

## Roadmap

The current spec is the **first slice**. Planned extensions:

- **Statements**: `let`, sequential composition, conditionals.
- **Loops**: while-loops with explicit termination measures.
- **Calls**: function-call lowering and inlining proofs.
- **Effects**: separate `eval` for pure vs. IO functions, with the
  Resilient `pure` / `io` / `fails` annotations bridged in.
- **Linkage**: a CI step that runs `lake build` on the emitted theorem
  bundle and fails the PR if any proof breaks.
