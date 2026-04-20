---
id: RES-237
title: "L0001 false positives: lint walkers miss Assume, MapLiteral, SetLiteral, LetDestructureStruct"
state: DONE
priority: P2
goalpost: G10
created: 2026-04-20
owner: executor
Claimed-by: Claude
Closed-by: 0d080de
---

## Summary

The `collect_identifier_reads_in` and `collect_lets_in` helper
functions in `resilient/src/lint.rs` (lines ~145–291) do not handle
several AST node variants that exist in the language. This causes
two classes of false-positive L0001 "unused local binding" warnings:

1. **Variables used only in `assume()` conditions are reported as
   unused.** Example:

   ```
   fn f(int x) -> Int {
       assume(x > 0);  // x is "used" here …
       return x;       // … and here, but lint ignores assume()
   }
   ```

   Because `collect_identifier_reads_in` has no arm for
   `Node::Assume { condition, .. }`, identifiers referenced inside
   an `assume` condition are invisible to the lint.

2. **Variables used only in map or set literals are reported as
   unused.**

   ```
   fn f(int k, int v) {
       let m = { k: v };   // k, v missed by the lint walker
   }
   ```

   `Node::MapLiteral { entries, .. }` and
   `Node::SetLiteral { items, .. }` have no arm in
   `collect_identifier_reads_in`.

3. **`LetDestructureStruct` field-bindings are never collected.**
   `collect_lets_in` has no arm for
   `Node::LetDestructureStruct { fields, .. }`, so struct-
   destructure bindings are never considered for L0001 analysis
   (neither flagged when unused, nor marked used when read).

## Acceptance criteria

- Add `Node::Assume { condition, .. }` to `collect_identifier_reads_in`:
  recurse into `condition`.
- Add `Node::MapLiteral { entries, .. }` to
  `collect_identifier_reads_in`: recurse into each entry's key and
  value expressions.
- Add `Node::SetLiteral { items, .. }` to
  `collect_identifier_reads_in`: recurse into each item.
- Add `Node::LetDestructureStruct { value, fields, .. }` to both
  `collect_lets_in` (record each field binding name + span) and
  `collect_identifier_reads_in` (recurse into `value`).
- Unit tests (new, not modifying existing tests):
  - `assume(x > 0)` in a fn body where `x` is also read → no L0001.
  - `let m = {k: v};` where `k` and `v` are also read → no L0001.
  - Struct-destructure binding that is never read → L0001 fires.
  - Struct-destructure binding that is read → no L0001.
- `cargo test` remains fully green.
- `cargo clippy --all-targets -- -D warnings` remains clean.
- Commit message: `RES-237: fix L0001 false positives for Assume, MapLiteral, SetLiteral, LetDestructureStruct`.

## Affected code

- `resilient/src/lint.rs` — `collect_identifier_reads_in` (~line 186)
  and `collect_lets_in` (~line 148).

## Notes

- Do **not** modify any existing tests — only add new ones.
- The fix is purely additive (new `match` arms); no behavioural
  change to existing arms is required.
- A similar gap may exist in the `walk_*` helpers for other lint
  passes (L0002–L0005); a follow-up audit is worth doing but is
  out of scope for this ticket.

## Log
- 2026-04-20 created by analyzer (found during static review of lint.rs)
