#!/usr/bin/env bash
# RES-368: build the WASM playground for static hosting.
#
# Produces a self-contained `playground/dist/` directory:
#   - index.html, style.css, main.js (copied from `playground/web/`)
#   - pkg/ (wasm-bindgen output from `wasm-pack`)
#   - examples.json (manifest of `resilient/examples/*.rz` and *.res
#     files, baked at build time so the page does not need a server-
#     side examples endpoint)
#
# Exit codes:
#   0 — dist/ is built and within the size budget
#   1 — wasm-pack missing
#   2 — build failed
#   3 — gzipped .wasm exceeds the size budget
#
# Usage:
#   playground/build.sh               # build, no size enforcement
#   playground/build.sh --check-size  # also enforce ≤ 2 MiB gzip

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$SCRIPT_DIR/dist"
SIZE_LIMIT=$((2 * 1024 * 1024))

CHECK_SIZE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check-size) CHECK_SIZE=1; shift ;;
    *) echo "unknown flag: $1" >&2; exit 2 ;;
  esac
done

if ! command -v wasm-pack >/dev/null 2>&1; then
  echo "error: wasm-pack not found on PATH; install with 'cargo install wasm-pack'" >&2
  exit 1
fi

echo ">>> building WASM crate"
cd "$SCRIPT_DIR"
wasm-pack build \
  --target web \
  --release \
  --out-dir "$DIST_DIR/pkg" \
  --out-name resilient_playground

echo ">>> copying static assets"
cp "$SCRIPT_DIR/web/index.html" "$DIST_DIR/index.html"
cp "$SCRIPT_DIR/web/style.css"  "$DIST_DIR/style.css"
cp "$SCRIPT_DIR/web/main.js"    "$DIST_DIR/main.js"

echo ">>> baking examples.json from resilient/examples/"
python3 - "$REPO_ROOT" "$DIST_DIR/examples.json" <<'PYEOF'
import json
import os
import sys

repo_root = sys.argv[1]
out_path = sys.argv[2]

examples_dir = os.path.join(repo_root, "resilient", "examples")
items = []
for name in sorted(os.listdir(examples_dir)):
    if not (name.endswith(".rz") or name.endswith(".res")):
        continue
    path = os.path.join(examples_dir, name)
    try:
        with open(path, encoding="utf-8") as f:
            source = f.read()
    except (UnicodeDecodeError, OSError):
        continue
    # Skip the very large examples — the dropdown is not the place
    # to scroll a 500-line snippet. Keeps the manifest under a few
    # hundred KB.
    if len(source) > 16 * 1024:
        continue
    items.append({"name": name, "source": source})

with open(out_path, "w", encoding="utf-8") as f:
    json.dump(items, f, indent=2)
print(f"baked {len(items)} examples into {out_path}")
PYEOF

if (( CHECK_SIZE )); then
  echo ">>> checking gzipped .wasm size"
  WASM_FILE="$DIST_DIR/pkg/resilient_playground_bg.wasm"
  if [ ! -f "$WASM_FILE" ]; then
    echo "error: expected wasm output at $WASM_FILE" >&2
    exit 2
  fi
  GZIPPED=$(gzip -c "$WASM_FILE" | wc -c | tr -d ' ')
  echo "    gzipped size: $GZIPPED bytes (limit: $SIZE_LIMIT bytes)"
  if (( GZIPPED > SIZE_LIMIT )); then
    echo "error: WASM artifact exceeds 2 MiB gzip budget" >&2
    exit 3
  fi
fi

echo ">>> dist/ ready: $DIST_DIR"
