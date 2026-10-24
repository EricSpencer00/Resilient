#!/usr/bin/env python3
"""Observe independent source-review fixtures using the hosted candidate only."""
import hashlib
import json
import os
import pathlib
import subprocess
import sys
binary = pathlib.Path(sys.argv[1]).resolve()
output = pathlib.Path(sys.argv[2]).resolve()
output.mkdir(parents=True, exist_ok=True)
fixtures = pathlib.Path(__file__).parent / "review-fixtures"
review = json.loads((fixtures / "source-review.json").read_text())
records = []
for name, expected_hash in review["files"].items():
    source = fixtures / name
    assert hashlib.sha256(source.read_bytes()).hexdigest() == expected_hash
    case = output / source.stem
    case.mkdir()
    (case / "input.rz").write_bytes(source.read_bytes())
    env = dict(os.environ)
    env.pop("RESILIENT_OVERFLOW_MODE", None)
    result = subprocess.run([str(binary), "--no-cache", "--emit-contract-certificate", str(case / "report.json"), str(source)], env=env, capture_output=True, timeout=30)
    (case / "stdout.txt").write_bytes(result.stdout)
    (case / "stderr.txt").write_bytes(result.stderr)
    assert b"panicked" not in result.stderr, (name, result.stderr)
    value = json.loads((case / "report.json").read_text())
    clauses = [clause for function in value["functions"] for clause in function["clauses"] if clause["kind"] == "ensures"]
    if name in ("parameter_result_positive.rz", "signed_div_rem_control.rz"):
        assert result.returncode == 0, (name, result.stderr)
        assert all(clause["basis"] == "implementation" and clause["verdict"] == "pass" and "BitVec 64" in clause["smtlib2"] for clause in clauses), (name, value)
        expected = ["6"] if name.startswith("parameter") else ["-2", "-1"]
        assert [line for line in result.stdout.decode().splitlines() if line.lstrip("-").isdigit()] == expected
    elif name == "parameter_result_negative.rz":
        assert result.returncode != 0 and clauses[0]["verdict"] == "fail", value
    else:
        assert all(not (clause["basis"] == "implementation" and clause["verdict"] == "pass") for clause in clauses), (name, value)
    records.append({"fixture": name, "source_sha256": expected_hash, "exit_code": result.returncode, "ensures": [{"basis": c["basis"], "verdict": c["verdict"]} for c in clauses]})
(output / "observations.json").write_text(json.dumps({"compiler_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(), "source_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(), "review_is_external_acceptance": False, "cases": records}, sort_keys=True, indent=2) + "\n")
print(f"Recorded {len(records)} reviewed-source observations; unsupported cases remain withheld")
