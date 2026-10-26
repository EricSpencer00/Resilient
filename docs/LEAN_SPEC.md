---
layout: page
title: "Lean-Proven Semantics"
nav_order: 31
permalink: /lean-spec/
---

# Lean-Proven Semantics

Resilient includes a small operational semantics in **Lean 4**, in
[`resilient/lean-spec/`](https://github.com/EricSpencer00/Resilient/tree/main/resilient/lean-spec).
It models expressions, and the compiler can emit theorem source for selected
single-return functions. Emitting source is not a successful Lean proof check.
A checked theorem relates the emitted expression to this model; it does not
establish correctness of a compiled binary.

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
  simp [eval, env_of, Env.lookup, double]
```

Build the model and explicitly check the generated module:

```bash
# From the repository root; rz must already be built.
python3 docs/verify_lean_spec.py --compiler resilient/target/debug/rz
```

The checker runs `lake build`, emits `double` and `triple` from
`resilient/examples/lean_spec_demo.rz`, and runs `lake env lean` on each
output. It fails on missing tools, failed commands, timeouts, missing theorem
declarations, and explicit `sorry`, `admit`, or `axiom` admissions in emitted
source. The Lean toolchain is pinned by `lean-toolchain`; install Lake before
running the command. Dependencies and the model remain part of the trusted base.

For another function, save its emitted source to `Generated.lean`, build the
library, then run `lake env lean /absolute/path/to/Generated.lean` from
`resilient/lean-spec/`. Merely placing a file in a `Generated/` directory and
running `lake build` does not show that it was checked: the default library
root imports only `AST`, `Semantics`, and `Theorems`.

## What a function must look like to lower

For the first slice, `--emit-lean-spec` accepts:

- A single `return EXPR;` statement
- `EXPR` is built from: integer/bool literals, parameter identifiers,
  `+ - * / %`, `== < <= > >=`, unary `-` and `!`

Functions with loops, calls, struct accesses, or multi-statement bodies
yield an `unsupported` diagnostic. Each restriction lifts as the Lean
fragment grows.

## Proof boundary

`eval` uses Lean's unbounded `Int`, with booleans represented as 0/1.
It does not model machine-width overflow, memory, hardware IO, actor scheduling,
or the compiler's VM, JIT, and embedded code generators. The current lowering
does not accept conditionals even though the Lean AST includes `ite`.
Boolean literals, boolean parameters and comparisons also need correspondence
work: the native theorem renderer uses `Bool`, while `eval` returns `Option Int`.
Division and remainder need explicit domain conditions and agreement on signed
arithmetic before generated theorems can be relied on for those expressions.

The small lemmas above establish properties of `eval`. They do not prove that
the Rust optimizer applies only sound transformations or that generated
instructions implement `eval`. No source-to-machine-code refinement theorem
is supplied by this project. These artifacts alone do not qualify the compiler
under a safety standard.

See [Formal-methods coverage](../formal-methods/) for the boundaries of the
Z3, Lean, and TLA+ mechanisms and the remaining assurance gaps.

## Roadmap

The current spec is the **first slice**. Planned extensions:

- **Statements**: `let`, sequential composition, conditionals.
- **Loops**: while-loops with explicit termination measures.
- **Calls**: function-call lowering and inlining proofs.
- **Effects**: separate `eval` for pure vs. IO functions, with the
  Resilient `pure` / `io` / `fails` annotations bridged in.
- **Linkage**: run the explicit generated-module checker in CI and retain
  the tool versions, source revision, checked modules, and results. The
  standalone checker above does not add a required CI gate.
