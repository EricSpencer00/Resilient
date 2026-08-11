---
name: Good first issue
about: A well-scoped task suitable for a first-time contributor.
title: "<area>: <what is wrong, in plain words>"
labels: ["good first issue", "ticket"]
assignees: ""
---

<!--
Thanks for filing a good-first-issue! These should be:
  - Well-scoped (a few hours of focused work at most)
  - Self-contained (no deep dependency on other in-flight work)
  - Clearly described (someone unfamiliar with the codebase can pick it up)

Before filing, search [open issues](https://github.com/EricSpencer00/Resilient/issues?q=is%3Aissue+is%3Aopen) to confirm this isn't already tracked.

TITLE: say what is wrong in plain words. `STDLIB.md: document the 18
complex_* builtins` invites a stranger; `RES-3145: probabilistic_contracts
check should validate declarations` does not. Do not put a RES-NNNN id in
the title — that belongs on the branch and in the commit message.

THE THREE RULES THAT DECIDE WHETHER ANYONE PICKS THIS UP:

  1. Every claim you make must come from a command you actually ran, with
     the output pasted. Not "this is probably undocumented" — run it,
     paste the count.
  2. File references are REPO-RELATIVE with line numbers
     (`resilient/src/lib.rs:12630`). Never paste an absolute path from
     your own machine; it tells the reader nobody proofread this.
  3. State the trap up front — the one non-obvious thing that will make a
     good-faith attempt fail. If you know it and don't write it down, you
     are spending someone else's afternoon to save yourself a sentence.

An issue written this way is what produced this project's first outside
contribution. An auto-generated one has never produced any.
-->

## Context

_What is this ticket about, in one or two sentences? Link to any related
tickets, code, or docs._

## Goal

_What should the state of the codebase be when this ticket is closed?_

## Reproduce it

_A copy-pasteable snippet or shell loop that shows the problem, together
with its **verified** output. Say so explicitly — "output above verified
against `./resilient/target/debug/rz`" — so the reader knows it was run
and not guessed._

## The trap

_The one non-obvious thing that will make a good-faith attempt fail.
e.g. "`documented` must be a word-boundary match, not a substring match,
or `int_to_str` will falsely match `int_to_str_radix`." Delete this
section only if you are certain there is no such trap._

## Acceptance criteria

_At least one must be a literal command someone can run._

- [ ] _e.g. `cargo test --manifest-path resilient/Cargo.toml --test it examples_golden` is green_
- [ ] _Criterion 2 — something observable, not a description of effort_
- [ ] _Criterion 3_

## Hints for the contributor

- **Where to start**: _file / module / function the contributor should open
  first._
- **Where to add tests**: _e.g. a unit test in `resilient/src/lib.rs`, a
  focused integration test under `resilient/tests/`, or a new
  `resilient/examples/foo.rz` with a `foo.expected.txt` sidecar._
- **Relevant docs**: _links to SYNTAX.md / STABILITY.md / ROADMAP.md
  sections that explain surrounding context._

## Out of scope

_Anything explicitly NOT part of this ticket — helps keep the PR focused._

---

New to the project? Read
[CONTRIBUTING.md](../CONTRIBUTING.md) first — it covers dev setup, how to
claim a ticket, and the PR checklist.
