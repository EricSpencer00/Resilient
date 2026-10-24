#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PYTHON="${PYTHON:-python3}"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

# A throwaway repository, so the red and green runs can differ by a real
# implementation commit without touching the checkout this script lives in.
REPO="$WORK_DIR/repo"
mkdir -p "$REPO/agent-scripts" "$REPO/tests"
cp "$SCRIPT_DIR/e2e-evidence.py" "$SCRIPT_DIR/e2e-attestation-probe.py" "$REPO/agent-scripts/"
RUNNER="$REPO/agent-scripts/e2e-evidence.py"
PROBE="agent-scripts/e2e-attestation-probe.py"
FIXTURE="tests/regression_case.txt"
OTHER_FIXTURE="tests/other_fixture.txt"
printf 'regression fixture\n' > "$REPO/$FIXTURE"
printf 'other fixture\n' > "$REPO/$OTHER_FIXTURE"
git -C "$REPO" init -q
# A run's identity covers its source revision but not its outcome, so every
# variant that must differ from another run gets a revision of its own.
commit() {
  git -C "$REPO" add -A
  git -C "$REPO" -c user.name=e2e -c user.email=e2e@example.invalid commit -q --allow-empty -m "$1"
}
commit "regression test and fixture"

# run_case OUTPUT TEST OUTCOME [FIXTURE]: records a probe run. The probe's
# behaviour is driven by environment variables, which are not recorded, so the
# recorded command is identical for the red and green runs.
run_case() {
  local output="$1" test="$2" outcome="$3" fixture="${4:-$FIXTURE}"
  set +e
  E2E_ATTEST_TEST="$test" E2E_ATTEST_OUTCOME="$outcome" \
    "$PYTHON" "$RUNNER" run --output "$output" --input "$fixture" --input "$PROBE" \
    -- "$PYTHON" "$PROBE" >/dev/null 2>&1
  local status=$?
  set -e
  [[ "$status" -ne 2 ]] || { echo "run failed to write evidence: $output" >&2; exit 1; }
  "$PYTHON" "$RUNNER" verify "$output" >/dev/null
}

# expect_reject PATTERN COMMAND...: the command must fail with PATTERN on stderr.
expect_reject() {
  local pattern="$1"
  shift
  local error status
  set +e
  error="$("$@" 2>&1 >/dev/null)"
  status=$?
  set -e
  if [[ "$status" -eq 0 ]]; then
    echo "unexpectedly accepted: $*" >&2
    exit 1
  fi
  if ! grep -q -- "$pattern" <<<"$error"; then
    echo "rejected for the wrong reason (expected '$pattern'): $error" >&2
    exit 1
  fi
}

RED="$WORK_DIR/red"
run_case "$RED" regression_case FAILED

printf 'fixed\n' > "$REPO/implementation.txt"
commit "implement the regression fix"
GREEN="$WORK_DIR/green"
run_case "$GREEN" regression_case ok

ATTEST="$WORK_DIR/attestation"
"$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$GREEN" --output "$ATTEST"
"$PYTHON" "$RUNNER" verify-attestation "$ATTEST" --baseline "$RED" --final "$GREEN"

python3 - "$ATTEST/attestation.json" "$RED/manifest.json" "$GREEN/manifest.json" <<'PY'
import json, pathlib, sys
attestation = json.loads(pathlib.Path(sys.argv[1]).read_text())
red = json.loads(pathlib.Path(sys.argv[2]).read_text())
green = json.loads(pathlib.Path(sys.argv[3]).read_text())
assert attestation["schema"] == "resilient-e2e-red-green-attestation"
assert attestation["test"] == "regression_case"
assert attestation["command"] == green["command"] == red["command"]
assert attestation["inputs"] == green["inputs"] == red["inputs"]
assert attestation["toolchain"] == green["toolchain"]
assert attestation["baseline"]["run_id"] == red["run_id"]
assert attestation["final"]["run_id"] == green["run_id"]
assert red["source"]["commit"] != green["source"]["commit"]
assert attestation["baseline"]["manifest"]["result"]["status"] == "failed"
assert attestation["final"]["manifest"]["result"]["status"] == "passed"
assert {"name": "regression_case", "status": "failed"} in red["result"]["outcomes"]
assert {"name": "regression_case", "status": "passed"} in green["result"]["outcomes"]
PY

REPEAT="$WORK_DIR/attestation-repeat"
"$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$GREEN" --output "$REPEAT" >/dev/null
[[ "$(cat "$ATTEST/checksums.sha256")" == "$(cat "$REPEAT/checksums.sha256")" ]]
cmp -s "$ATTEST/attestation.json" "$REPEAT/attestation.json"

# A baseline that passes is not a red result.
expect_reject "does not record regression_case as a test failure" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$GREEN" --final "$RED" --output "$WORK_DIR/x1"

# A failed build, with no test outcome, is not a red result.
BUILD="$WORK_DIR/build-failure"
commit "build failure revision"
set +e
E2E_ATTEST_BUILD=1 E2E_ATTEST_TEST=regression_case E2E_ATTEST_OUTCOME=FAILED \
  "$PYTHON" "$RUNNER" run --output "$BUILD" --input "$FIXTURE" --input "$PROBE" -- "$PYTHON" "$PROBE" >/dev/null 2>&1
set -e
expect_reject "does not record regression_case as a test failure" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$BUILD" --final "$GREEN" --output "$WORK_DIR/x2"

# Another test failing in the baseline means the red run is not the target regression.
OTHER_FAIL="$WORK_DIR/other-failure"
commit "other failure revision"
set +e
E2E_ATTEST_EXTRA=unrelated_case E2E_ATTEST_TEST=regression_case E2E_ATTEST_OUTCOME=FAILED \
  "$PYTHON" "$RUNNER" run --output "$OTHER_FAIL" --input "$FIXTURE" --input "$PROBE" -- "$PYTHON" "$PROBE" >/dev/null 2>&1
set -e
expect_reject "baseline fails outside the target test: unrelated_case" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$OTHER_FAIL" --final "$GREEN" --output "$WORK_DIR/x3"

# A final run that does not record the target test is rejected.
SILENT="$WORK_DIR/silent-final"
commit "silent revision"
set +e
E2E_ATTEST_SILENT=1 E2E_ATTEST_TEST=regression_case E2E_ATTEST_OUTCOME=ok \
  "$PYTHON" "$RUNNER" run --output "$SILENT" --input "$FIXTURE" --input "$PROBE" -- "$PYTHON" "$PROBE" >/dev/null 2>&1
set -e
expect_reject "final run does not record regression_case passing" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$SILENT" --output "$WORK_DIR/x4"

# A different test name in the final run is a mismatched regression.
MISMATCH="$WORK_DIR/mismatched-test"
commit "mismatched test revision"
run_case "$MISMATCH" other_case ok
expect_reject "final run does not record regression_case passing" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$MISMATCH" --output "$WORK_DIR/x5"

# Different test fixtures in the two runs are rejected.
FIXTURE_CHANGED="$WORK_DIR/fixture-changed"
commit "fixture changed revision"
run_case "$FIXTURE_CHANGED" regression_case ok "$OTHER_FIXTURE"
expect_reject "test or fixture inputs differ" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$FIXTURE_CHANGED" --output "$WORK_DIR/x6"

expect_reject "same run" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$RED" --final "$RED" --output "$WORK_DIR/x7"

ALTERED_RED="$WORK_DIR/altered-red"
cp -R "$RED" "$ALTERED_RED"
printf 'altered\n' >> "$ALTERED_RED/summary.md"
expect_reject "summary does not match the manifest" \
  "$PYTHON" "$RUNNER" attest --test regression_case --baseline "$ALTERED_RED" --final "$GREEN" --output "$WORK_DIR/x8"

ALTERED_ATTEST="$WORK_DIR/altered-attestation"
cp -R "$ATTEST" "$ALTERED_ATTEST"
printf 'altered\n' >> "$ALTERED_ATTEST/summary.md"
expect_reject "attestation summary does not match" \
  "$PYTHON" "$RUNNER" verify-attestation "$ALTERED_ATTEST"

RETARGETED="$WORK_DIR/retargeted-attestation"
cp -R "$ATTEST" "$RETARGETED"
sed 's/"regression_case"/"other_case"/' "$ATTEST/attestation.json" > "$RETARGETED/attestation.json"
expect_reject "attestation" \
  "$PYTHON" "$RUNNER" verify-attestation "$RETARGETED"

expect_reject "bundle does not match the attestation" \
  "$PYTHON" "$RUNNER" verify-attestation "$ATTEST" --baseline "$GREEN" --final "$GREEN"

printf 'E2E red-green attestation passed: red/green acceptance, determinism, and rejection of invalid evidence.\n'
