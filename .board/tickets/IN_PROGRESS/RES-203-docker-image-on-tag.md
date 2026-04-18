---
id: RES-203
title: Publish Docker image on git tag push
state: IN_PROGRESS
priority: P3
goalpost: infra
created: 2026-04-17
owner: executor
---

## Summary
Users who want to try Resilient without installing Rust should be
able to `docker run ghcr.io/ericspencer00/resilient:latest
--help`. Small ticket: a Dockerfile and a GitHub Actions workflow
that publishes on tag.

## Acceptance criteria
- `Dockerfile` at repo root:
  - Multi-stage: builder image with rust:1.84, runtime with
    debian:bookworm-slim + z3 + libz3-4.
  - Final stage ENTRYPOINT `/usr/local/bin/resilient`.
- `.github/workflows/release_image.yml` triggered on `push` tags
  matching `v*` — builds for linux/amd64 + linux/arm64, pushes
  to `ghcr.io/ericspencer00/resilient:{tag, latest}`.
- README "Install" section gets a "Docker" subsection.
- Commit message: `RES-203: docker image published on tag`.

## Notes
- Z3 binary is the bulk of the runtime image. Accept the weight
  for now; shipping without Z3 loses the `--features z3` surface.
- Multi-arch via `docker/build-push-action@v5` with buildx
  setup — straightforward, well-documented path.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
