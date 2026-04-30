# Self-Hosting Parser — Implementation Plan

**Date:** 2026-04-30
**Status:** Design lock-in for [#171 RES-379](https://github.com/EricSpencer00/Resilient/issues/171)
**Predecessor:** RES-323 (`self-host/lexer.rz` — the lexer is already self-hosted; this ticket is the parser step)

---

## TL;DR

This is the largest open ticket in the backlog by a wide margin. Writing the Resilient parser in Resilient itself requires:

1. **Sum types (RES-400, in flight)** — the parser's AST is naturally tagged-enum-shaped. Without payload-carrying enums, the implementation goes through workaround representations (string-tagged structs) that bloat the code several-fold.
2. **First-class function values (RES-403, ✅ closed)** — needed for the recursive-descent driver's parser-combinator-style helpers.
3. **Polymorphic Array<T> (RES-402, ✅ surface in)** — needed for `Vec<Token>`, `Vec<Node>`, etc.
4. **Generic monomorphization (RES-405, design landed)** — without this, every helper is duplicated per element type.
5. **Closures with capture (RES-164c/d, open)** — recursive-descent parsers naturally close over the input cursor.
6. **A full string library** — character classification, span manipulation, formatted output — most of which the existing stdlib ships.

The ticket's acceptance criterion is "the parser written in Resilient can parse `resilient/examples/*.rz` and produce the same AST as the Rust-side parser, modulo span/diagnostic fidelity." That's a 10k+ LOC translation of the existing `parse_function`/`parse_statement`/`parse_expression` family.

## Estimated effort

A motivated contributor familiar with both Resilient's surface and the existing Rust-side parser could ship this in **6–10 weeks** of focused work, sequenced as:

- **Weeks 1–2**: token reader + small grammar (literals, identifiers, basic expressions). Land alongside a snapshot test that compares its output to the Rust parser's on a curated set of golden examples.
- **Weeks 3–4**: function/let/struct/enum declarations.
- **Weeks 5–6**: full expression parser with operator precedence, including the harder bits (lambda literals, struct literals with named fields, match expressions, the question-mark operator).
- **Weeks 7–8**: error recovery — match the Rust parser's `record_error` + `synchronize` semantics so diagnostics are equivalent.
- **Weeks 9–10**: contracts (`requires` / `ensures`), live blocks, actor declarations, traits, generics. Ship the equivalence test against every example in `resilient/examples/`.

## Why this is blocked

The chain above means that until **RES-400 (sum types) ships PRs 4–6** (match patterns + exhaustiveness + interpreter eval), the parser's natural AST representation is not expressible in Resilient. Writing the parser around the workaround hurts so much that the ticket explicitly carries the `blocked` tag.

The recommended sequence:

1. Land RES-400 PRs 4–6 (sum types ⇒ pattern match ⇒ exhaustiveness ⇒ interpreter).
2. Land RES-405 PRs 1–4 (generics implementation).
3. Land closure capture (RES-164c/d).
4. *Then* schedule the self-hosting work as 5 sub-PRs on a dedicated agent session.

## Why this is the right ticket to write down but not implement today

Self-hosting matters because it's the cleanest demonstration that the language is expressive enough to describe itself — a real safety-critical signal. But the ticket is a *capstone*, not a first step. Pushing it forward without the prerequisite chain produces code that doesn't represent how a self-hosted parser would be written when the language is mature. Write the doc; revisit when the prereqs land.

## What this PR doesn't do

- No code changes. The existing self-hosted lexer (`self-host/lexer.rz`) is unchanged.
- No partial parser scaffold. Starting before the prereqs would lock in workarounds that have to be deleted.

## What this PR does

- This document. It captures the implementation plan and the prerequisite chain so when the prereqs land an agent can pick up the parser ticket and execute the 5-PR sequence above without re-deriving the dependencies.
- A clear "schedule when X lands" note at the top of [#171](https://github.com/EricSpencer00/Resilient/issues/171) — see the issue's comment thread for the link.
