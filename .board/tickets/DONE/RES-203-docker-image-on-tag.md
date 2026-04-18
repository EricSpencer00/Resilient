---
id: RES-203
title: Publish Docker image on git tag push
state: DONE
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

## Resolution

### Files added
- `Dockerfile` at repo root. Two-stage:
  - **builder**: `rust:1.85-bookworm` + `libz3-dev` + `clang` +
    `pkg-config`. Runs
    `cargo build --release --manifest-path resilient/Cargo.toml
     --features z3 --locked`.
  - **runtime**: `debian:bookworm-slim` + `libz3-4` +
    `ca-certificates`. Copies the release binary to
    `/usr/local/bin/resilient`. Creates an unprivileged
    `resilient` user (UID 1001) and sets `WORKDIR
    /home/resilient`. `ENTRYPOINT ["/usr/local/bin/resilient"]`.
- `.dockerignore` — excludes `.git`, `.github`, `.board`,
  every `target/`, editor droppings, the `vscode-extension`
  Node build output, the fuzz crate's `target/corpus/artifacts`,
  and host-specific native bench binaries so the build context
  stays minimal and cache-friendly.
- `.github/workflows/release_image.yml` — triggers on `push`
  tags matching `v*` and on manual dispatch. Standard
  buildx-multi-arch flow:
  - `docker/setup-qemu-action@v3` + `docker/setup-buildx-action@v3`.
  - `docker/login-action@v3` against `ghcr.io` using the
    workflow's `GITHUB_TOKEN` (job has `packages: write`).
  - `docker/metadata-action@v5` derives tags:
    - `{{version}}` and `{{major}}.{{minor}}` from the semver
      git tag.
    - `latest` ONLY on stable tags (no `-pre`).
    - `manual-<sha>` on `workflow_dispatch` so dry-runs don't
      clobber `latest`.
  - `docker/build-push-action@v5` builds `linux/amd64` +
    `linux/arm64` from the repo-root Dockerfile, with
    GHA-backed layer cache (`cache-from`/`cache-to: type=gha`).
    `push:` is gated on `push` or `workflow_dispatch`.

### Files changed
- `README.md` — new "Docker (RES-203)" subsection at the top
  of "Getting Started" with a `docker run` quickstart, a
  mount-and-run pattern, and notes on the multi-arch image,
  `--features z3` baseline, and the non-root user.

### Design deviations
- **Base image is `rust:1.85-bookworm`, not `rust:1.84`.** The
  ticket's AC names 1.84, but every crate in this workspace
  uses `edition = "2024"`, which requires Rust 1.85+ (edition
  2024 stabilized in Rust 1.85, Feb 2025). Pinning 1.84 would
  break the build. 1.85 is the minimum that satisfies both
  the ticket's intent (a specific pinned rust version) and
  the codebase's edition.

- **Runtime image adds `ca-certificates`.** Not in the AC but
  a common defensive add — zero size impact, buys future
  HTTPS-adjacent work (e.g. downloading external crates /
  cert manifests) without a base-image bump.

- **Runs as unprivileged user.** Ticket doesn't mention,
  but Docker best practice. README documents that users
  mounting writable volumes may need matching UID 1001.

### Verification
- `cargo test --locked` in the resilient crate → unchanged
  (pure additive at the repo root; no Rust source touched).
- `ruby -ryaml` on the workflow YAML → parses clean.
- Dockerfile syntax hand-validated (no `hadolint` available
  locally). End-to-end `docker build` NOT run locally: the
  Docker daemon isn't running on the dev host. The build
  will be exercised in CI on the first tag push.

### Follow-ups (not in this ticket)
- **Image-size trimming.** The runtime image carries libz3-4
  (~30MB) + glibc. Removing z3 (for a `features =
  default`-only image) would halve the size; worth a separate
  ticket once users split into "verified" vs "interp-only"
  buckets.
- **Image signing.** cosign + the Ed25519 story from RES-194
  could eventually sign the image itself, closing the loop
  between cert provenance and binary provenance.
- **`rust:1.84` deviation resolution.** Manager: if the AC's
  1.84 was specific (rather than "any recent pinned 1.x"),
  flag it; otherwise the 1.85 bump is the right call.

## Log
- 2026-04-17 created by manager
- 2026-04-17 claimed by executor
- 2026-04-17 resolved by executor (multi-stage Dockerfile with
  rust:1.85-bookworm → debian:bookworm-slim + libz3-4,
  multi-arch ghcr.io publish workflow, README quickstart)
