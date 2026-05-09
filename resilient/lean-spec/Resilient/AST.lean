/-!
# Resilient.AST — the Resilient AST in Lean

This is the mirror of the Resilient compiler's pure-arithmetic AST
fragment. The Rust emitter (see `resilient/src/lean_spec.rs`) translates
Resilient functions whose body is a single `return` of a pure
arithmetic-or-conditional expression into a `Expr` value here.

The fragment is deliberately small — loops, calls, and structs are
*not* part of the Lean spec for the first slice. Adding them is a
matter of extending this file plus the Rust lowering pass.
-/

namespace Resilient

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
  deriving Repr

end Resilient
