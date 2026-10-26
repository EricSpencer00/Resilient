import Resilient.Semantics
import Init.Data.Int.DivModLemmas

/-! Truncating expression arithmetic. These lemmas do not model i64 overflow. -/

namespace Resilient

theorem eval_div_int (env : Env) (a b : Int) (nonzero : b ≠ 0) :
    eval env (.div (.int a) (.int b)) = some (Int.tdiv a b) := by
  cases b with
  | ofNat n =>
    cases n with
    | zero => exact (nonzero rfl).elim
    | succ n => rfl
  | negSucc n => rfl

theorem eval_mod_int (env : Env) (a b : Int) (nonzero : b ≠ 0) :
    eval env (.mod (.int a) (.int b)) = some (Int.tmod a b) := by
  cases b with
  | ofNat n =>
    cases n with
    | zero => exact (nonzero rfl).elim
    | succ n => rfl
  | negSucc n => rfl

theorem eval_div_zero (env : Env) (a : Int) :
    eval env (.div (.int a) (.int 0)) = none := by
  rfl

theorem eval_mod_zero (env : Env) (a : Int) :
    eval env (.mod (.int a) (.int 0)) = none := by
  rfl

theorem truncating_quotient_remainder (a b : Int) :
    Int.tdiv a b * b + Int.tmod a b = a := by
  exact Int.tdiv_add_tmod' a b

theorem negative_nonmultiple_rounding :
    (eval [] (.div (.int (-7)) (.int 3)),
     eval [] (.mod (.int (-7)) (.int 3))) = (some (-2), some (-1)) := by
  decide

end Resilient
