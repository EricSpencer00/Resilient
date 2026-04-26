#!/usr/bin/env bash
# RES-219: one-line installer for the `rz` CLI.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/EricSpencer00/Resilient/main/scripts/install.sh | bash
#
# Optional environment variables:
#   RZ_VERSION   Tag to install (default: latest release).
#   RZ_PREFIX    Install prefix (default: $HOME/.rz; binary goes to $PREFIX/bin).
#                Set to /usr/local for a system-wide install (will sudo if needed).
#
# What this does:
#   1. Detects host OS + arch (Linux/macOS, x86_64/aarch64).
#   2. Downloads the matching `rz-<tag>-<target>.tar.gz` from the
#      GitHub release.
#   3. Extracts `rz` into <PREFIX>/bin and chmods +x.
#   4. Prints a one-liner the user can paste into their shell rc to
#      put <PREFIX>/bin on PATH.
#
# No sudo unless PREFIX is not writable. No background daemons. No
# package-manager touching. Removable with `rm <PREFIX>/bin/rz`.

set -euo pipefail

REPO="EricSpencer00/Resilient"
PREFIX="${RZ_PREFIX:-$HOME/.rz}"
BIN_DIR="$PREFIX/bin"

err() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[32m==>\033[0m %s\n' "$*"; }

# ----- detect host -----
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
    Linux)  os_tag="unknown-linux-gnu" ;;
    Darwin) os_tag="apple-darwin" ;;
    *) err "unsupported OS: $OS (Resilient ships Linux + macOS binaries today; Windows users — see CONTRIBUTING.md for source-build instructions)";;
esac

case "$ARCH" in
    x86_64|amd64)        arch_tag="x86_64" ;;
    arm64|aarch64)       arch_tag="aarch64" ;;
    *) err "unsupported arch: $ARCH";;
esac

TARGET="${arch_tag}-${os_tag}"

# ----- pick version -----
TAG="${RZ_VERSION:-}"
if [ -z "$TAG" ]; then
    info "resolving latest release..."
    TAG="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | \
           grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
    [ -n "$TAG" ] || err "could not resolve latest release tag (rate-limited? try RZ_VERSION=vX.Y.Z)"
fi

ARCHIVE="rz-${TAG}-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE}"

# ----- download + extract -----
TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

info "downloading $URL"
if ! curl -fSL --retry 3 -o "$TMPDIR/$ARCHIVE" "$URL"; then
    err "download failed — check that release $TAG has an asset for $TARGET"
fi

info "extracting"
tar xzf "$TMPDIR/$ARCHIVE" -C "$TMPDIR"
[ -f "$TMPDIR/rz" ] || err "archive missing rz binary"

# ----- install -----
mkdir -p "$BIN_DIR"
install -m 0755 "$TMPDIR/rz" "$BIN_DIR/rz" 2>/dev/null \
    || cp "$TMPDIR/rz" "$BIN_DIR/rz"
chmod +x "$BIN_DIR/rz"

info "installed: $BIN_DIR/rz ($("$BIN_DIR/rz" --version 2>/dev/null || echo unknown))"

# ----- PATH hint -----
case ":$PATH:" in
    *":$BIN_DIR:"*)
        info "$BIN_DIR is already on PATH — try: rz --help" ;;
    *)
        cat <<EOF

Add to your shell rc (~/.bashrc, ~/.zshrc, etc.):

    export PATH="$BIN_DIR:\$PATH"

Then reload (\`exec \$SHELL\`) and run: rz --help
EOF
        ;;
esac
