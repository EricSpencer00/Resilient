---
layout: page
title: "AI Threat Model"
nav_order: 30
permalink: /ai-threat-model/
---

# AI Threat Model

> *"The LLM is a client of the type system, not the prover."*

The historically named `--ai-threats` pass flags syntactic patterns for review
regardless of who wrote the code. Agent-written code can be excellent; human
authorship offers no guarantee. A warning identifies a pattern, not its author
or a proven defect. Absence of a warning is not evidence of complete assurance.

Reported confidence values are heuristics, not calibrated defect probabilities.
Use the [assurance provisions](ASSURANCE_PROVISIONS.md) to review requirements,
specification assumptions, implementation conformance and check coverage.

## The threat catalogue

| Kind | Pattern |
|---|---|
| `OffByOne` | `while i <= len(arr)` |
| `MissedElse` | `if c { return X; } CODE` with no `else` |
| `SwallowedError` | `catch _ { }` |
| `MagicNumber` | numeric literal > 1 outside known-good contexts |
| `CopyPasteBlock` | two ≥3-stmt blocks with identical AST shape |
| `UnboundedLoop` | `while true { ... }` no `break` |
| `GhostHandler` | catch arm contains only `println` / trivial return |
| `HallucinatedIdent` | call to identifier 1–2 edits from a builtin |
| `NestedConditional` | `if`-expressions nested ≥3 deep |
| `SilentSwallow` | failing try + literal-returning catch |

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
exhibits the configured patterns, regardless of provenance. This policy is
additional review enforcement; it does not prove the function correct.

## Why a "threat model"?

The pass supplies review prompts for possible failure modes such as swallowed
errors and nontermination. Pattern detection is a heuristic; correctness claims
need evidence about intended behavior and the actual implementation.
Deterministic tools also depend on their input specification, translation and
configuration. A checker can correctly accept a model that omits a requirement.

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
  counts missing annotation and call-reference signals.
- [`resilience_score`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/resilience_score.rs) —
  grades a function A–F across five axes.
- [`anti_regression`](https://github.com/EricSpencer00/Resilient/blob/main/resilient/src/anti_regression.rs) —
  locks a function's behavioral fingerprint with `#[stable(...)]`.

The AI threat model is the *prevention* counterpart to those *measurement* modules.
Together they provide assurance signals for code from any author.
