; implementation-profile: vm-i64-wrap-v1
; arithmetic-semantics: bytecode-vm; other-backends-unqualified
(set-logic QF_BV)
(declare-fun x () (_ BitVec 64))
(assert (let ((a!1 (=> (and (not (bvsge x #x0000000000000000))) true))
      (a!2 (=> (and (not (bvsge x #x0000000000000000))) (= x x))))
  (not (and true (=> true (and true a!1)) (=> true a!2)))))

(check-sat)
