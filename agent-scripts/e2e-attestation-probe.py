import os
import sys

test = os.environ["E2E_ATTEST_TEST"]
outcome = os.environ["E2E_ATTEST_OUTCOME"]
extra = os.environ.get("E2E_ATTEST_EXTRA")
if os.environ.get("E2E_ATTEST_BUILD") == "1":
    print("error: could not compile `resilient`", file=sys.stderr)
    raise SystemExit(101)
if os.environ.get("E2E_ATTEST_SILENT") == "1":
    raise SystemExit(0)
if extra:
    print(f"test {extra} ... FAILED")
print(f"test {test} ... {outcome}")
raise SystemExit(101 if outcome == "FAILED" or extra else 0)
