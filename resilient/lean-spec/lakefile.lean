import Lake
open Lake DSL

package «resilient-spec» where
  -- Resilient operational semantics in Lean 4.
  -- Build with `lake build` from this directory.

@[default_target]
lean_lib «Resilient» where
  -- The Resilient namespace contains:
  --   Resilient.AST        — the AST mirror
  --   Resilient.Semantics  — big-step evaluation
  --   Resilient.Theorems   — proven lemmas
  roots := #[`Resilient]
