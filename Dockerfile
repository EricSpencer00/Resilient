# RES-203/RES-3946/RES-3947: Docker image for the Resilient compiler
# + runtime + MCP server.
#
# Multi-stage build:
#   - builder:  rust:1.90-bookworm + libz3-dev + clang — compiles
#               `resilient/target/release/rz` with
#               `--features z3,lsp`, then strips debug symbols.
#   - runtime:  debian:bookworm-slim + the z3 CLI package (which
#               pulls in libz3-4) — carries the stripped binary,
#               the z3 solver `resilient_verify`/`--z3` shell out
#               to, and a HEALTHCHECK against the MCP HTTP wrapper.
#
# The resulting image's ENTRYPOINT is the `rz` binary, so
# `docker run ghcr.io/ericspencer00/resilient:latest --help` (and
# `rz src/main.rs`) "just work". Run the MCP server with
# `docker run -p 8080:8080 ... mcp --http-port 8080`.
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
        --features z3,lsp \
        --locked

# RES-3947: strip debug symbols from the release binary before it
# crosses into the runtime stage — smaller COPY, smaller final image.
RUN strip --strip-all resilient/target/release/rz

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

# MCP registry namespace ownership marker. The Resilient compiler ships
# an MCP server (`rz --mcp`) and registers under this name on
# registry.modelcontextprotocol.io. The registry verifies ownership of
# Docker images by matching this label against the published server name.
# Note: case-sensitive — the GitHub OIDC subject carries the original
# case of the username, so the namespace here must match exactly.
LABEL io.modelcontextprotocol.server.name="io.github.EricSpencer00/resilient"

# RES-3946: `z3` ships the CLI binary (`/usr/bin/z3`) that
# `rz verify-all --z3` and the MCP `resilient_verify` tool shell
# out to — Debian's `z3` package links it statically, so it does
# NOT pull in libz3-4 as a dependency. `libz3-4` is listed
# explicitly for the *shared* lib the release binary itself
# dynamically linked against (via libz3-dev in the builder stage).
# ca-certificates is a defensive add for anyone piping certs
# through the binary later; small and useful.
# curl backs the HEALTHCHECK below. `--no-install-recommends`
# keeps the final image lean.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      z3 \
      libz3-4 \
      ca-certificates \
      curl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /src/resilient/target/release/rz /usr/local/bin/rz

# RES-3946: Z3_BINARY lets deployments point at a differently-named
# or non-PATH z3 install; defaulted here to the CLI installed above
# so `resilient_verify` works out of the box. Override at `docker run`
# time with `-e Z3_BINARY=/path/to/z3` if needed.
ENV Z3_BINARY=/usr/bin/z3

EXPOSE 8080

# Default to a non-root user — Docker best practice; also
# prevents accidental writes to host-mounted volumes as root.
RUN useradd --create-home --shell /bin/bash --uid 1001 resilient
USER resilient
WORKDIR /home/resilient

# RES-3947: liveness/readiness probe for the MCP HTTP wrapper. Only
# meaningful when the container is run as `rz mcp --http-port 8080`;
# harmless (and simply unused) for other `rz` invocations.
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -f http://127.0.0.1:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/rz"]
