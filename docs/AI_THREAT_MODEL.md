---
layout: page
title: "AI Threat Model"
nav_order: 30
permalink: /ai-threat-model/
---

# AI Threat Model

> *"The LLM is a client of the type system, not the prover."*

Resilient is the first production language whose type system carries an explicit
threat model for its own contributors. The premise: most Resilient code today is
written or assisted by an LLM, and we don't trust the LLM. We don't trust humans
unconditionally either, but humans get other tooling. This module catches the
*patterns that mark code as AI-generated-without-careful-review*.

The list is deliberately conservative. We only flag patterns whose
false-positive rate sits below 5% across the standard corpus
(`resilient/examples/` + the in-tree test suite).

## The threat catalogue

| Kind | Pattern | False-positive rate |
|---|---|---|
| `OffByOne` | `while i <= len(arr)` | < 1% |
| `MissedElse` | `if c { return X; } CODE` with no `else` | ~3% |
| `SwallowedError` | `catch _ { }` | < 1% |
| `MagicNumber` | numeric literal > 1 outside known-good contexts | ~5% |
| `CopyPasteBlock` | two ≥3-stmt blocks with identical AST shape | ~4% |
| `UnboundedLoop` | `while true { ... }` no `break` | < 1% |
| `GhostHandler` | catch arm contains only `println` / trivial return | ~3% |
| `HallucinatedIdent` | call to identifier 1–2 edits from a builtin | ~2% |
| `NestedConditional` | `if`-expressions nested ≥3 deep | ~4% |
| `SilentSwallow` | failing try + literal-returning catch | < 1% |

## Two surfaces

### 1. Soft pass — `--ai-threats`

```bash
rz --ai-threats my_file.rz
```

Prints every detected threat with `file:line:col`, kind, description, confidence,
and a one-line mitigation suggestion. Exits 0 even with violations — this is the
exploration mode for working through a codebase.

Example output:

```
my_file.rz: 3 AI threat(s) detected
  in fn `process_buffer`: [off-by-one] while loop bounded by `<= len(...)`,
    likely off-by-one (confidence=85%) — use a half-open range and `< len(...)`
  in fn `dispatch`: [swallowed-error] empty `catch` arm — error is silently
    dropped (confidence=95%) — either re-raise the error or annotate the
    function `fails`
  in fn `compute`: [magic-number] integer literal `7919` in arithmetic — name it
    (confidence=55%) — name the constant via `let` or `const`
```

### 2. Hard gate — `#[ai_review_required]`

```rz
#[ai_review_required]
fn safety_critical(int x) -> int {
    return x + x;     // OK
}

#[ai_review_required]
fn careless(int n) -> int {
    while true { }    // ERROR: unbounded-loop in #[ai_review_required] fn
    return 0;
}
```

When a function carries `#[ai_review_required]`, every threat detected in its
body is promoted to a **hard compile error**. Use the attribute on functions
that flow into safety-critical paths so the type system refuses code that
exhibits AI-style failure modes.

## Why a "threat model"?

The framing is deliberate. Most lints describe a *style preference* ("prefer
`match` over nested `if`"). The AI threat model describes *adversarial inputs* —
patterns that systematically appear in code generated under specific conditions
(token-by-token greedy decoding, context-window exhaustion, prompt drift).

Naming them as a threat model makes them tractable:

- A *style preference* is negotiable per-team.
- A *threat* is an enemy to be defeated.

Resilient's type system is built around the principle that we trust the
verifier and not the producer. The AI threat model is the natural extension
of that principle to the code itself.

## Roadmap

The current pass is the **first slice** — 10 detections, all syntactic. Follow-up
PRs will deepen each one:

- **OffByOne**: extend to for-loops, range bounds, and `arr.len() - 1` patterns.
- **HallucinatedIdent**: build a per-project name index instead of relying on
  the standard-builtin shortlist.
- **CopyPasteBlock**: cluster across functions, not just within one.
- **SilentTypeCoerce**: add a precision-loss detection pass once the type
  inferer carries enough type information to lattice-compare.

## See also

- [`vibe_debt`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/vibe_debt.rs) —
  measures the *gap* between asserted and provable correctness.
- [`resilience_score`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/resilience_score.rs) —
  grades a function A–F across five axes.
- [`anti_regression`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/anti_regression.rs) —
  locks a function's behavioral fingerprint with `#[stable(...)]`.

The AI threat model is the *prevention* counterpart to those *measurement* modules.
Together they form the vibe-coded-resilience pipeline.
