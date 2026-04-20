---
id: RES-219
title: GitHub release workflow — native binary builds for Linux + macOS
status: DONE
labels: [infra, release]
roadmap: G20
Claimed-by: Claude Sonnet 4.6
---

## Goal

When a maintainer pushes a semver tag (`vMAJOR.MINOR.PATCH`) to `main`, GitHub
Actions should automatically build native `resilient` binaries for four
platforms and attach them to a new GitHub Release with auto-generated release
notes.

## Platforms

| Target | OS | Archive |
|---|---|---|
| `x86_64-unknown-linux-gnu` | ubuntu-latest | `.tar.gz` |
| `aarch64-unknown-linux-gnu` | ubuntu-latest (via `cross`) | `.tar.gz` |
| `x86_64-apple-darwin` | macos-latest | `.tar.gz` |
| `aarch64-apple-darwin` | macos-latest | `.tar.gz` |

## Files to touch

- `.github/workflows/release.yml` — create this file
- `CONTRIBUTING.md` — add a "Releases" section explaining how to cut a release

## Acceptance criteria

- [x] `release.yml` triggers on `v*` tag pushes
- [x] All four platform binaries build successfully
- [x] A GitHub Release is created with the tag name as the title
- [x] Each `.tar.gz` is attached to the release as a downloadable asset
- [x] `CONTRIBUTING.md` explains the tag → release workflow
- [x] The workflow does NOT fail for `workflow_dispatch` dry-runs

## Out of scope

- crates.io publishing
- Windows binaries
- Code signing

## Closing notes

Implemented in commit f5668fe (branch `main`). Ticket closed by this PR.
Closing commit: 694c4c1
