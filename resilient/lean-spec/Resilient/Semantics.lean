/-!
# Resilient.Semantics — big-step evaluation

We define a partial big-step evaluator `eval : Env → Expr → Option Int`
that yields `none` on a type mismatch (e.g. adding an `int` and a
`bool`) or a divide-by-zero. The `Env` is an association list keyed
by parameter name.

The evaluator is structural recursion on `Expr`, so it is total in
the Lean sense — `eval` always returns, but the `Option` reflects
that some Resilient expressions don't reduce to an integer.
-/

import Resilient.AST

namespace Resilient

/-- An association-list environment from variable names to integers. -/
def Env := List (String × Int)

/-- Build an `Env` from a list of bindings. -/
def env_of (bindings : List (String × Int)) : Env := bindings

/-- Look up a name in an environment; returns the first match. -/
def Env.lookup : Env → String → Option Int
  | [], _ => none
  | (k, v) :: rest, x => if k = x then some v else Env.lookup rest x

/-- Big-step evaluation of an `Expr` to an `Option Int`. Booleans
    are encoded as 0/1 in this fragment. -/
def eval (env : Env) : Expr → Option Int
  | .int n => some n
  | .bool true => some 1
  | .bool false => some 0
  | .var x => Env.lookup env x
  | .add l r =>
    match eval env l, eval env r with
    | some a, some b => some (a + b)
    | _, _ => none
  | .sub l r =>
    match eval env l, eval env r with
    | some a, some b => some (a - b)
    | _, _ => none
  | .mul l r =>
    match eval env l, eval env r with
    | some a, some b => some (a * b)
    | _, _ => none
  | .div l r =>
    match eval env l, eval env r with
    | some _, some 0 => none
    | some a, some b => some (a / b)
    | _, _ => none
  | .mod l r =>
    match eval env l, eval env r with
    | some _, some 0 => none
    | some a, some b => some (a % b)
    | _, _ => none
  | .eq l r =>
    match eval env l, eval env r with
    | some a, some b => some (if a = b then 1 else 0)
    | _, _ => none
  | .lt l r =>
    match eval env l, eval env r with
    | some a, some b => some (if a < b then 1 else 0)
    | _, _ => none
  | .le l r =>
    match eval env l, eval env r with
    | some a, some b => some (if a ≤ b then 1 else 0)
    | _, _ => none
  | .neg e =>
    match eval env e with
    | some a => some (-a)
    | none => none
  | .not_ e =>
    match eval env e with
    | some 0 => some 1
    | some _ => some 0
    | none => none
  | .ite c t e =>
    match eval env c with
    | some 0 => eval env e
    | some _ => eval env t
    | none => none

end Resilient
