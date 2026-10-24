#!/usr/bin/env bash
# Install a checksum-verified release without a compiler or package manager.
# curl -fsSL https://raw.githubusercontent.com/EricSpencer00/Resilient/main/scripts/install.sh | bash
# RZ_VERSION pins a release; RZ_PREFIX defaults to ~/.rz. RZ_PROFILE overrides
# shell startup files for managed environments. No elevated privileges are used.
set -euo pipefail

REPO="EricSpencer00/Resilient"
PREFIX="${RZ_PREFIX:-$HOME/.rz}"
case "$PREFIX" in /*) ;; *) PREFIX="$PWD/$PREFIX" ;; esac
BIN_DIR="$PREFIX/bin"
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }
case "$BIN_DIR" in *$'\n'*|*:* ) err 'the install path cannot contain newlines or colons' ;; esac

case "$(uname -s)" in
    Linux) os_tag="unknown-linux-gnu" ;;
    Darwin) os_tag="apple-darwin" ;;
    *) err 'release binaries support Linux and macOS' ;;
esac
case "$(uname -m)" in
    x86_64|amd64) arch_tag="x86_64" ;;
    arm64|aarch64) arch_tag="aarch64" ;;
    *) err 'release binaries support x86_64 and arm64' ;;
esac
TARGET="${arch_tag}-${os_tag}"
TAG="${RZ_VERSION:-}"
if [ -z "$TAG" ]; then
    TAG="$(curl -fsSL --retry 3 "https://api.github.com/repos/${REPO}/releases/latest" |
        sed -nE 's/.*"tag_name":[[:space:]]*"([^"]+)".*/\1/p')"
fi
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-+][A-Za-z0-9.-]+)?$ ]] || err 'could not resolve a valid release tag; set RZ_VERSION to a published version'
ARCHIVE="rz-${TAG}-${TARGET}.tar.gz"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
DOWNLOAD_DIR="$(mktemp -d)"
STAGED_BINARY=""
trap 'rm -rf "$DOWNLOAD_DIR"; [ -z "$STAGED_BINARY" ] || rm -f "$STAGED_BINARY"' EXIT

info "Downloading Resilient $TAG for $TARGET"
curl -fsSL --retry 3 -o "$DOWNLOAD_DIR/$ARCHIVE" "$BASE_URL/$ARCHIVE" || err "no downloadable release asset for $TARGET at $TAG"
curl -fsSL --retry 3 -o "$DOWNLOAD_DIR/SHA256SUMS" "$BASE_URL/SHA256SUMS" || err 'release checksum file could not be downloaded'
EXPECTED="$(awk -v archive="$ARCHIVE" '$2 == archive { print $1 }' "$DOWNLOAD_DIR/SHA256SUMS")"
[[ "$EXPECTED" =~ ^[0-9a-fA-F]{64}$ ]] || err 'release checksum is missing or ambiguous'
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL="$(sha256sum "$DOWNLOAD_DIR/$ARCHIVE" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
    ACTUAL="$(shasum -a 256 "$DOWNLOAD_DIR/$ARCHIVE" | awk '{print $1}')"
else
    err 'this system has no SHA-256 utility (sha256sum or shasum)'
fi
[ "$ACTUAL" = "$EXPECTED" ] || err 'download checksum does not match; installation unchanged'
CONTENTS="$(tar -tzf "$DOWNLOAD_DIR/$ARCHIVE")" || err 'invalid release archive'
case "$CONTENTS" in rz|./rz) ;; *) err 'release archive must contain only the rz executable' ;; esac
tar -xzf "$DOWNLOAD_DIR/$ARCHIVE" -C "$DOWNLOAD_DIR"
[ -f "$DOWNLOAD_DIR/rz" ] && [ ! -L "$DOWNLOAD_DIR/rz" ] || err 'release archive does not contain a regular rz executable'
chmod 0755 "$DOWNLOAD_DIR/rz"
VERSION="$("$DOWNLOAD_DIR/rz" --version)" || err 'this release executable cannot run on your system; installation unchanged'
case "$VERSION" in
    "rz ${TAG#v}"|"rz ${TAG#v}:"*|"rz ${TAG#v} "*) ;;
    *) err 'downloaded executable version does not match the release; installation unchanged' ;;
esac

# Validate shell destinations before replacing any existing installation.
printf -v QUOTED_BIN_DIR '%q' "$BIN_DIR"
PATH_LINE="case \":\$PATH:\" in *:$QUOTED_BIN_DIR:*) ;; *) export PATH=$QUOTED_BIN_DIR:\"\$PATH\" ;; esac"
PROFILES=()
if [ -n "${RZ_PROFILE:-}" ]; then
    PROFILES=("$RZ_PROFILE")
else
    case "${SHELL:-}" in
        */zsh) PROFILES=("$HOME/.zshrc" "$HOME/.zprofile") ;;
        *) PROFILES=("$HOME/.bashrc" "$HOME/.profile")
           if [ -f "$HOME/.bash_profile" ]; then PROFILES+=("$HOME/.bash_profile"); fi ;;
    esac
fi
for profile in "${PROFILES[@]}"; do
    mkdir -p "$(dirname "$profile")" || err "cannot prepare shell startup directory; installation unchanged"
    if [ -e "$profile" ] || [ -L "$profile" ]; then
        [ -f "$profile" ] && [ -w "$profile" ] || err "shell startup file is not writable: $profile; installation unchanged"
    else
        [ -w "$(dirname "$profile")" ] || err "shell startup directory is not writable; installation unchanged"
    fi
done

# Stage on the destination filesystem so replacing an existing executable is atomic.
mkdir -p "$BIN_DIR"
STAGED_BINARY="$(mktemp "$BIN_DIR/.rz-install.XXXXXX")"
cp "$DOWNLOAD_DIR/rz" "$STAGED_BINARY"
chmod 0755 "$STAGED_BINARY"
mv -f "$STAGED_BINARY" "$BIN_DIR/rz"
STAGED_BINARY=""

# A child installer cannot change its caller's environment. Configure future
# shells automatically, and reuse an existing writable PATH directory when possible.
for profile in "${PROFILES[@]}"; do
    if [ ! -f "$profile" ] || ! grep -Fqx "$PATH_LINE" "$profile"; then
        printf '\n# Resilient installer\n%s\n' "$PATH_LINE" >> "$profile" || err "rz was installed at $BIN_DIR/rz, but shell configuration failed: $profile"
    fi
done

AVAILABLE_NOW=false
case ":$PATH:" in *":$BIN_DIR:"*) AVAILABLE_NOW=true ;; esac
if [ "$AVAILABLE_NOW" = false ] && [ -z "${RZ_PROFILE:-}" ]; then
    IFS=: read -r -a PATH_DIRECTORIES <<< "$PATH"
    for directory in "${PATH_DIRECTORIES[@]}"; do
        case "$directory" in "$HOME"/*)
            if [ -d "$directory" ] && [ -w "$directory" ] && [ ! -e "$directory/rz" ] && [ ! -L "$directory/rz" ]; then
                ln -s "$BIN_DIR/rz" "$directory/rz"
                AVAILABLE_NOW=true
                break
            fi ;;
        esac
    done
fi
info "Installed $VERSION at $BIN_DIR/rz"
if [ "$AVAILABLE_NOW" = true ]; then
    info 'Ready: rz --help'
else
    info 'Shell configured. Open a new terminal to use rz.'
fi
