#!/usr/bin/env bash
# Preserve the documented entry point and RESILIENT_BIN override.
set -eu
exec python3 "$(cd "$(dirname "$0")" && pwd)/verify_tutorial_snippets.py" "$@"
