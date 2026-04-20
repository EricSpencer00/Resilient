---
id: RES-219
title: GitHub release workflow — native binary builds for Linux + macOS
status: OPEN
labels: [infra, release]
roadmap: G20
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

- [ ] `release.yml` triggers on `v*` tag pushes
- [ ] All four platform binaries build successfully
- [ ] A GitHub Release is created with the tag name as the title
- [ ] Each `.tar.gz` is attached to the release as a downloadable asset
- [ ] `CONTRIBUTING.md` explains the tag → release workflow
- [ ] The workflow does NOT fail for `workflow_dispatch` dry-runs

## Out of scope

- crates.io publishing
- Windows binaries
- Code signing
