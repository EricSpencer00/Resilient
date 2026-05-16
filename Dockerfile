# RES-203: Docker image for the Resilient compiler + runtime.
#
# Multi-stage build:
#   - builder:  rust:1.85-bookworm + libz3-dev + clang — compiles
#               `resilient/target/release/rz` with `--features z3`.
#   - runtime:  debian:bookworm-slim + libz3-4 — carries just the
#               compiled binary and the shared-library dep that
#               Z3's runtime needs.
#
# The resulting image's ENTRYPOINT is the `rz` binary, so
# `docker run ghcr.io/ericspencer00/resilient:latest --help` (and
# `rz src/main.rs`) "just work".
#
# Builder base: rust:1.90-bookworm. Edition 2024 (used by every crate
# in this workspace) requires Rust 1.85+, and one of our transitive
# deps (`home@0.5.12`) requires 1.88+, so bumping past 1.88 is necessary
# to avoid `rustc is not supported by the following package` errors.
# 1.90 is the most recent stable bookworm tag.

# ---------- builder ----------
FROM rust:1.90-bookworm AS builder

# libz3-dev provides the Z3 headers + libz3.so for linking; clang
# is needed by some sys-crate build scripts (curve25519-dalek's
# optional backend, z3 crate, etc.). apt-get clean trims layer
# size — we don't ship the builder image, but the habit keeps
# CI caches smaller.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libz3-dev \
      clang \
      pkg-config \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Copy sources. `.dockerignore` keeps target/, .git, and local
# artifacts out of the build context so the `COPY` is fast.
COPY . .

# Build in release, with the z3 feature so the shipped binary has
# the SMT-backed verifier enabled. `--locked` pins against the
# committed Cargo.lock.
RUN cargo build \
        --release \
        --manifest-path resilient/Cargo.toml \
        --features z3 \
        --locked

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

# MCP registry namespace ownership marker. The Resilient compiler ships
# an MCP server (`rz --mcp`) and registers under this name on
# registry.modelcontextprotocol.io. The registry verifies ownership of
# Docker images by matching this label against the published server name.
LABEL io.modelcontextprotocol.server.name="io.github.ericspencer00/resilient"

# libz3-4 provides libz3.so.4 at the system-library path the
# binary linked against. ca-certificates is a defensive add for
# anyone piping certs through the binary later; small and
# useful. `--no-install-recommends` keeps the final image lean.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libz3-4 \
      ca-certificates \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/resilient/target/release/rz /usr/local/bin/rz

# Default to a non-root user — Docker best practice; also
# prevents accidental writes to host-mounted volumes as root.
RUN useradd --create-home --shell /bin/bash --uid 1001 resilient
USER resilient
WORKDIR /home/resilient

ENTRYPOINT ["/usr/local/bin/rz"]
