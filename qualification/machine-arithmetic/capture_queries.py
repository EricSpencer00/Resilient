#!/usr/bin/env python3
"""Sign actual CLI diagnostic queries; SAT witnesses are not proof certificates."""
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

binary, raw, output = map(lambda value: pathlib.Path(value).resolve(), sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
def run(*args):
    return subprocess.check_output(args, stderr=subprocess.PIPE, timeout=30)
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
records = []
with tempfile.TemporaryDirectory(prefix="machine-query-key-") as temporary:
    private = pathlib.Path(temporary) / "key.pem"
    run("openssl", "genpkey", "-algorithm", "ed25519", "-out", str(private))
    public = output / "public.pem"
    run("openssl", "pkey", "-in", str(private), "-pubout", "-out", str(public))
    for case in sorted(raw.iterdir()):
        report = case / "report.json"
        if not report.is_file():
            continue
        value = json.loads(report.read_text())
        for function in value["functions"]:
            for index, query in enumerate(function.get("implementation_queries", [])):
                assert value["implementation_profile"] == "vm-i64-wrap-v1"
                text = query["query"]
                assert "BitVec 64" in text and "(set-logic QF_BV)" in text
                assert text.count("(check-sat)") == 1
                path = output / f"{case.name}-{function['name']}-{index}.smt2"
                path.write_text(text)
                result = run("z3", "-smt2", str(path)).decode().strip()
                assert result == query["result"], (path, result, query)
                assert result in ("sat", "unsat")
                run("openssl", "pkeyutl", "-sign", "-inkey", str(private), "-rawin", "-in", str(path), "-out", str(path) + ".sig")
                run("openssl", "pkeyutl", "-verify", "-pubin", "-inkey", str(public), "-rawin", "-in", str(path), "-sigfile", str(path) + ".sig")
                records.append({"case": case.name, "function": function["name"], "clause": query["clause"], "path": query["path"], "program_sha256": digest(case / "input.rz"), "report_sha256": digest(report), "query": path.name, "sha256": digest(path), "replayed_result": result, "role": "proof" if result == "unsat" else "counterexample-diagnostic"})
    assert sum(record["replayed_result"] == "sat" for record in records) >= 4, records
    assert sum(record["replayed_result"] == "unsat" for record in records) >= 5, records
    record = {"profile": "vm-i64-wrap-v1", "other_backends_qualified": False, "source_commit": run("git", "rev-parse", "HEAD").decode().strip(), "source_tree": run("git", "rev-parse", "HEAD^{tree}").decode().strip(), "compiler_sha256": digest(binary), "solver_identity": json.loads((raw.parent / "solver.json").read_text()), "solver_identity_sha256": digest(raw.parent / "solver.json"), "queries": records}
    manifest = output / "query-review.json"
    manifest.write_text(json.dumps(record, sort_keys=True, indent=2) + "\n")
    run("openssl", "pkeyutl", "-sign", "-inkey", str(private), "-rawin", "-in", str(manifest), "-out", str(manifest) + ".sig")
    run("openssl", "pkeyutl", "-verify", "-pubin", "-inkey", str(public), "-rawin", "-in", str(manifest), "-sigfile", str(manifest) + ".sig")
print(f"Retained {len(records)} signed raw queries with independently matched SAT/UNSAT outcomes")
