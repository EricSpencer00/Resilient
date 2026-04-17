---
id: RES-174
title: JIT cache: memoize compiled functions keyed by AST hash
state: OPEN
priority: P3
goalpost: G15
created: 2026-04-17
owner: executor
---

## Summary
Re-running a program re-lowers the same AST into the same
Cranelift IR into the same machine code. Wasteful. Hash the
function AST (post-typecheck) and cache the compiled fn-pointer
across runs within a session. Cross-session persistence is a
follow-up.

## Acceptance criteria
- `JitCache { map: HashMap<u64, FnPtr> }` on the JIT module.
- Hash: FNV-1a over a canonical serialization of the function AST
  (post-span stripping — spans shouldn't affect the cache).
- On `jit_compile(fn)`: hash; if hit, reuse; if miss, compile and
  store.
- Cache stats surfaced via `--jit-cache-stats` — prints
  `hits / misses / compiles` on exit.
- Unit test: call the JIT compile twice on the same function;
  second call reports a cache hit.
- Commit message: `RES-174: in-memory JIT cache keyed by AST hash`.

## Notes
- Cross-session cache (disk) is tempting but requires a stable
  serialization of Cranelift output, and invalidation on compiler
  version changes. Separate ticket.
- Thread safety: the JIT is single-threaded for now (no
  concurrent compile). Document the assumption.

## Log
- 2026-04-17 created by manager
