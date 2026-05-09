/-!
# Resilient.Theorems — proven correctness lemmas

The first three theorems we ship:

* `eval_int_lit_id` — `eval env (Expr.int n) = some n`
* `eval_add_comm` — addition is commutative under `eval`
* `eval_const_fold_sound` — folding `Add (Int a) (Int b)` to `Int (a+b)`
  preserves `eval`

These are all small but they're the foundation: every additional
optimisation pass in the Rust compiler can be discharged against the
same `eval` relation.
-/

import Resilient.AST
import Resilient.Semantics

namespace Resilient

/-- Integer literals evaluate to themselves. -/
theorem eval_int_lit_id (env : Env) (n : Int) :
    eval env (Expr.int n) = some n := by
  rfl

/-- Addition is commutative under big-step evaluation. -/
theorem eval_add_comm (env : Env) (a b : Expr) :
    eval env (Expr.add a b) = eval env (Expr.add b a) := by
  unfold eval
  cases eval env a with
  | none =>
    cases eval env b with
    | none => rfl
    | some _ => rfl
  | some va =>
    cases eval env b with
    | none => rfl
    | some vb =>
      simp [Int.add_comm]

/-- Constant folding `Add (Int a) (Int b)` ↝ `Int (a+b)` is sound. -/
theorem eval_const_fold_sound (env : Env) (a b : Int) :
    eval env (Expr.add (Expr.int a) (Expr.int b))
      = eval env (Expr.int (a + b)) := by
  rfl

/-- Negation is involutive: -(-x) = x. -/
theorem eval_neg_involutive (env : Env) (e : Expr) :
    eval env (Expr.neg (Expr.neg e)) = eval env e := by
  unfold eval
  cases eval env e with
  | none => rfl
  | some v => simp

end Resilient
