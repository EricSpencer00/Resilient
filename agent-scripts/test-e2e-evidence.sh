#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PYTHON="${PYTHON:-python3}"
RUNNER="$SCRIPT_DIR/e2e-evidence.py"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
FIXTURE="agent-scripts/test-e2e-evidence.sh"
ARTIFACT="$WORK_DIR/passing"

"$PYTHON" "$RUNNER" run \
  --output "$ARTIFACT" \
  --input "$FIXTURE" \
  -- python3 -c 'print("test evidence_smoke ... ok")'
"$PYTHON" "$RUNNER" verify "$ARTIFACT"

python3 - "$ARTIFACT/manifest.json" <<'PY'
import json, pathlib, sys
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert manifest["schema"] == "resilient-e2e-evidence"
assert manifest["schema_version"] == 1
assert manifest["source"]["commit"]
assert manifest["command"] == ["python3", "-c", 'print("test evidence_smoke ... ok")']
assert manifest["inputs"][0]["path"] == "agent-scripts/test-e2e-evidence.sh"
assert len(manifest["inputs"][0]["sha256"]) == 64
assert manifest["result"]["status"] == "passed"
assert manifest["result"]["exit_code"] == 0
assert {"name": "evidence_smoke", "status": "passed"} in manifest["result"]["outcomes"]
assert manifest["toolchain"]["python"].startswith("Python ")
PY

first_hash="$(cat "$ARTIFACT/checksums.sha256")"
"$PYTHON" "$RUNNER" run \
  --output "$ARTIFACT" \
  --input "$FIXTURE" \
  -- python3 -c 'print("test evidence_smoke ... ok")'
"$PYTHON" "$RUNNER" verify "$ARTIFACT"
second_hash="$(cat "$ARTIFACT/checksums.sha256")"
[[ "$first_hash" == "$second_hash" ]]

FAILED="$ARTIFACT"
set +e
"$PYTHON" "$RUNNER" run \
  --output "$FAILED" \
  --input "$FIXTURE" \
  -- python3 -c 'raise SystemExit(7)'
status=$?
set -e
[[ "$status" -eq 7 ]]
"$PYTHON" "$RUNNER" verify "$FAILED"
python3 - "$FAILED/manifest.json" <<'PY'
import json, pathlib, sys
manifest = json.loads(pathlib.Path(sys.argv[1]).read_text())
assert manifest["result"]["status"] == "failed"
assert manifest["result"]["exit_code"] == 7
PY

CORRUPT="$WORK_DIR/corrupt"
cp -R "$ARTIFACT" "$CORRUPT"
printf '{' > "$CORRUPT/manifest.json"
if "$PYTHON" "$RUNNER" verify "$CORRUPT"; then
  echo "malformed manifest unexpectedly verified" >&2
  exit 1
fi

TAMPERED="$WORK_DIR/tampered"
cp -R "$ARTIFACT" "$TAMPERED"
printf 'altered\n' >> "$TAMPERED/summary.md"
if "$PYTHON" "$RUNNER" verify "$TAMPERED"; then
  echo "altered evidence unexpectedly verified" >&2
  exit 1
fi

SECRET_ARTIFACT="$WORK_DIR/secret-output"
export RESILIENT_E2E_TEST_SECRET="EVIDENCE_SECRET_SENTINEL"
"$PYTHON" "$RUNNER" run \
  --output "$SECRET_ARTIFACT" \
  --input "$FIXTURE" \
  -- python3 -c 'import os; print(os.environ["RESILIENT_E2E_TEST_SECRET"])' >/dev/null
unset RESILIENT_E2E_TEST_SECRET
if grep -R -q "EVIDENCE_SECRET_SENTINEL" "$SECRET_ARTIFACT"; then
  echo "artifact unexpectedly contains environment data" >&2
  exit 1
fi

printf 'E2E evidence runner passed: pass/fail, repeatability, manifest validation, checksums, and secret exclusion.\n'
