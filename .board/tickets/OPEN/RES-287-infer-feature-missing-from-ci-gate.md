---
id: RES-287
title: "`--features infer` test suite is not exercised by any CI job"
state: OPEN
priority: P2
goalpost: G14
created: 2026-04-20
owner: executor
---

## Summary

The `infer` module is compiled only under `#[cfg(feature = "infer")]`
(see `main.rs` line 37). The CI workflow (`.github/workflows/ci.yml`)
runs `cargo test --locked` (default features) and
`cargo test --locked --features z3`, but has **no job for
`--features infer`**.

As a result:
- All tests in `resilient/src/infer.rs` (module `infer::tests`) are
  silently skipped on every CI run.
- At the time of writing, `cargo test --features infer` reports
  **3 failing tests** (2 are tracked by RES-259; 1 by RES-286).
- Future regressions to the inference engine will go undetected until
  someone manually runs `cargo test --features infer`.

## Affected file

- `.github/workflows/ci.yml` — add a CI step for the `infer` feature.

## Acceptance criteria

- `.github/workflows/ci.yml` gains a new build/test step:
  ```yaml
  - name: cargo test --features infer
    run: cargo test --locked --features infer
  ```
  placed alongside the existing `--features z3` step.
- The new step passes (i.e. all failures currently exposed under
  `--features infer` are fixed — see RES-259 and RES-286 — before or
  alongside this CI addition).
- `cargo test --features infer` passes with 0 failures locally before
  the PR is opened.
- `cargo clippy --all-targets -- -D warnings` clean.
- Commit: `RES-287: add --features infer CI gate`.

## Notes

- CLAUDE.md prohibits changes to `.github/workflows/` without explicit
  mention in the PR description and maintainer sign-off. Flag this in
  the PR.
- The `infer` feature has existed since the RES-120 prototype; the
  omission from CI is an oversight that grew as tests accumulated in
  `infer.rs`.
- Prerequisite: RES-259 (lint match-arm bindings) and RES-286 (parser
  underscore arm) should land first so the new CI step is green from
  day one.

## Log

- 2026-04-20 created by analyzer (no `--features infer` CI job; three
  test failures found under that feature flag)
